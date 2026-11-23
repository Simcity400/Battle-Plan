
use dashboard_base::products::Country;
use dashboard_base::products::{LoadContext, Product};
use dashboard_base::Source;
use dashboard_base::span::HourSpan;
use dashboard_query::{Query, identity::Holder, read};
use replay_data::rows::{MilitaryAidRow, TableId, TagHolderRow};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::atomic::AtomicBool,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

pub struct VolunteersExpeditionaries;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum MilitaryAidKind {
    Land,
    Expeditionary,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VolunteersExpeditionariesParams {
    pub branch_id: String,
    pub kind: MilitaryAidKind,
    #[doc = " Links touching these tags only; none for every link."]
    pub countries: Option<Vec<Country>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MilitaryAidRun {
    pub first_hour: i32,
    pub last_hour: i32,
    #[doc = " Null where not known; never zero for that."]
    pub divisions: Option<f64>,
    pub starts_segment: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MilitaryAidLink {
    pub key: String,
    pub sender: String,
    pub sender_slot: Option<u32>,
    pub sender_country_id: Option<u32>,
    pub host: String,
    pub host_slot: Option<u32>,
    pub host_country_id: Option<u32>,
    pub runs: Vec<MilitaryAidRun>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct VolunteersExpeditionariesReply {
    pub links: Vec<MilitaryAidLink>,
    #[doc = " The game's own tally of each sender's volunteer hosts (kind `land` only)."]
    pub tallies: Vec<MilitaryAidLink>,
    #[doc = " Hours whose aid is not known for every sender: the timeline holds none, or a list's sender"]
    #[doc = " resolves through no link."]
    pub unobserved: Vec<HourSpan>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(VolunteersExpeditionariesReply);

impl Product for VolunteersExpeditionaries {
    const KEY: &'static str = "volunteers-expeditionaries";
    const CONTENT_DEPENDENT: bool = false;
    type Params = VolunteersExpeditionariesParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, kind, countries, "countries[]"];
    type Reply = VolunteersExpeditionariesReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            VolunteersExpeditionariesParams::decl(),
            VolunteersExpeditionariesReply::decl(),
            MilitaryAidKind::decl(),
            MilitaryAidRun::decl(),
            MilitaryAidLink::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid aid branch")?;
        for tag in params.countries.iter().flatten() {
            dashboard_base::products::validate_country(tag)?;
        }
        Ok(())
    }

    fn load(source: &Source, params: &Self::Params, _context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| invalid("Invalid aid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let reply = aid(&query, params, |run| dashboard_base::queries::lifetimes(source, run, TableId::MilitaryAid).map(|held| held.to_vec()), cancelled)?;
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

type SlotHolders<'q> = HashMap<u32, Vec<(i32, i32, &'q str, u32)>>;

fn slot_holders<'q>(query: &'q Query<'_>) -> SlotHolders<'q> {
    let mut out: SlotHolders<'_> = HashMap::new();
    for (tag, runs) in &query.holders.tags {
        for run in runs {
            if let Holder::Found { slot, id } = run.holder {
                out.entry(slot).or_default().push((run.first, run.last, tag.as_str(), id));
            }
        }
    }
    for runs in out.values_mut() {
        runs.sort_unstable_by_key(|run| run.0);
    }
    out
}

fn key(sender: u32, host: u32, channel: u8) -> u128 {
    u128::from(sender) << 40 | u128::from(host) << 8 | u128::from(channel)
}

fn pairs(keys: &[u128]) -> Vec<(u32, u32)> {
    let mut out: Vec<(u32, u32)> = keys.iter().map(|&key| ((key >> 40) as u32, (key >> 8) as u32)).filter(|&(sender, host)| sender != TagHolderRow::NONE && host != TagHolderRow::NONE).collect();
    out.sort_unstable();
    out.dedup();
    out
}

fn lives(lives: &[(i64, i64)], hour: i32) -> bool {
    lives.iter().any(|&(born, gone)| born <= i64::from(hour) && i64::from(hour) <= gone)
}

fn value_at(runs: Option<&read::Runs>, hour: i32) -> Option<i64> {
    runs?.iter().find(|run| run.0 <= hour && hour <= run.1)?.2
}

fn within(spans: &[(i32, i32)], hour: i32) -> bool {
    spans.get(spans.partition_point(|span| span.1 < hour)).is_some_and(|span| span.0 <= hour)
}

fn pair_value(pair_runs: Option<&read::Runs>, available: &[(i32, i32)], unknown: bool, hour: i32) -> Option<i64> {
    if !within(available, hour) {
        return None;
    }
    match value_at(pair_runs, hour) {
        Some(value) => Some(value),
        None if unknown => None,
        None => Some(0),
    }
}

fn clamped(hour: i64) -> i32 {
    hour.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

fn aid(query: &Query<'_>, params: &VolunteersExpeditionariesParams, lifetimes: impl Fn(u32) -> io::Result<Vec<(u128, i64, i64)>>, cancelled: &AtomicBool) -> io::Result<VolunteersExpeditionariesReply> {
    let channel = match params.kind {
        MilitaryAidKind::Land => MilitaryAidRow::VOLUNTEERS,
        MilitaryAidKind::Expeditionary => MilitaryAidRow::EXPEDITIONARIES,
    };
    let chosen = |slot: u32, tag: &str| params.countries.as_ref().is_none_or(|countries| countries.iter().any(|country| dashboard_world::products::holder_tags::matches(country, (slot, 0), tag)));
    let holders = slot_holders(query);
    let divisions = summary_source()?;
    let recording = query.replay.recording();
    let mut reply = VolunteersExpeditionariesReply { links: Vec::new(), tallies: Vec::new(), unobserved: Vec::new(), warnings: Vec::new() };
    for gap in &query.timeline.unheld {
        reply.unobserved.push(HourSpan { first_hour: gap.first, last_hour: gap.last });
    }
    let mut links: BTreeMap<String, MilitaryAidLink> = BTreeMap::new();
    let mut tallies: BTreeMap<String, MilitaryAidLink> = BTreeMap::new();
    let mut lived: BTreeSet<String> = BTreeSet::new();
    for stretch in &query.timeline.stretches {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let lives_of = lifetimes(stretch.run)?;
        let meets = |&&(_, born, gone): &&(u128, i64, i64)| born <= i64::from(stretch.last) && gone >= i64::from(stretch.first);
        let channels: Vec<u8> = if channel == MilitaryAidRow::VOLUNTEERS { vec![channel, MilitaryAidRow::TALLIES] } else { vec![channel] };
        let keys: Vec<u128> = lives_of.iter().filter(meets).map(|&(key, ..)| key).filter(|&key| channels.contains(&(key as u8))).collect();
        if keys.is_empty() {
            continue;
        }
        let runs = read::runs(recording, stretch.run, TableId::MilitaryAid, divisions, Some(&keys), stretch.first, stretch.last)?;
        let available = dashboard_query::cost::storage(|| replay_lanes::lanes::available_reads(recording, stretch.run, TableId::MilitaryAid, stretch.first, stretch.last))?;
        let mut lifetime: HashMap<u128, Vec<(i64, i64)>> = HashMap::new();
        for &(key, born, gone) in lives_of.iter().filter(meets) {
            lifetime.entry(key).or_default().push((born, gone));
        }
        let life = |key: u128| lifetime.get(&key).map_or(&[][..], Vec::as_slice);
        let pairs = pairs(&keys);
        for &channel in &channels {
            let nobody = life(self::key(TagHolderRow::NONE, TagHolderRow::NONE, channel));
            if channel != MilitaryAidRow::TALLIES {
                for &(born, gone) in nobody {
                    let (first, last) = (clamped(born).max(stretch.first), clamped(gone).min(stretch.last));
                    if first <= last {
                        reply.unobserved.push(HourSpan { first_hour: first, last_hour: last });
                    }
                }
            }
            for &(sender, host) in &pairs {
                let pair = self::key(sender, host, channel);
                let unknown = life(self::key(sender, TagHolderRow::NONE, channel));
                let pair_runs = runs.get(&pair);
                let value = |hour: i32| pair_value(pair_runs, &available, lives(unknown, hour) || lives(nobody, hour), hour);
                for &(sender_first, sender_last, sender_tag, sender_id) in holders.get(&sender).into_iter().flatten() {
                    for &(host_first, host_last, host_tag, host_id) in holders.get(&host).into_iter().flatten() {
                        if !chosen(sender, sender_tag) && !chosen(host, host_tag) {
                            continue;
                        }
                        let lo = sender_first.max(host_first).max(stretch.first);
                        let hi = sender_last.min(host_last).min(stretch.last);
                        if lo > hi {
                            continue;
                        }
                        let mut edges = vec![lo];
                        for run in pair_runs.into_iter().flatten() {
                            edges.extend([run.0, run.1.saturating_add(1)]);
                        }
                        for &(born, gone) in unknown.iter().chain(nobody) {
                            edges.extend([clamped(born), clamped(gone + 1)]);
                        }
                        for &(first, last) in &available {
                            edges.extend([first, last.saturating_add(1)]);
                        }
                        edges.retain(|&edge| lo <= edge && edge <= hi);
                        edges.sort_unstable();
                        edges.dedup();
                        let pair_key = format!("{sender}:{sender_id}:{host}:{host_id}:{sender_tag}:{host_tag}");
                        let held = if channel == MilitaryAidRow::TALLIES { &mut tallies } else { &mut links };
                        let link = held.entry(pair_key.clone()).or_insert_with(|| MilitaryAidLink {
                            key: format!("{channel}:{pair_key}"),
                            sender: sender_tag.to_owned(),
                            sender_slot: Some(sender),
                            sender_country_id: Some(sender_id),
                            host: host_tag.to_owned(),
                            host_slot: Some(host),
                            host_country_id: Some(host_id),
                            runs: Vec::new(),
                        });
                        for (index, &start) in edges.iter().enumerate() {
                            let end = edges.get(index + 1).map_or(hi, |next| next - 1);
                            let divisions = value(start).map(|value| value as f64);
                            if value_at(pair_runs, start).is_some() {
                                lived.insert(pair_key.clone());
                            }
                            match link.runs.last_mut() {
                                Some(run) if start != lo && run.last_hour + 1 == start && run.divisions == divisions => run.last_hour = end,
                                previous => {
                                    let after_gap = start == lo || previous.is_none_or(|run| run.last_hour + 1 != start || run.divisions.is_none());
                                    link.runs.push(MilitaryAidRun { first_hour: start, last_hour: end, divisions, starts_segment: after_gap && divisions.is_some() });
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    reply.links = links.into_iter().filter(|(pair, _)| lived.contains(pair)).map(|(_, link)| link).collect();
    reply.tallies = tallies.into_iter().filter(|(pair, _)| lived.contains(pair)).map(|(_, link)| link).collect();
    reply.unobserved.sort_unstable_by_key(|span| span.first_hour);
    let mut merged: Vec<HourSpan> = Vec::new();
    for span in reply.unobserved.drain(..) {
        match merged.last_mut() {
            Some(held) if i64::from(span.first_hour) <= i64::from(held.last_hour) + 1 => held.last_hour = held.last_hour.max(span.last_hour),
            _ => merged.push(span),
        }
    }
    reply.unobserved = merged;
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pair_without_a_row_is_none_only_where_the_table_was_available_and_its_list_known() {
        let available = [(10, 19), (30, 39)];
        let runs: read::Runs = vec![(10, 14, Some(3)), (15, 19, None)];
        assert_eq!(pair_value(Some(&runs), &available, false, 12), Some(3));
        assert_eq!(pair_value(Some(&runs), &available, false, 16), Some(0), "the row gone while the table was available: none");
        assert_eq!(pair_value(None, &available, false, 25), None, "the table not available: not known, never zero");
        assert_eq!(pair_value(None, &available, false, 35), Some(0));
        assert_eq!(pair_value(None, &available, true, 35), None, "the sender's list not known: not known");
        assert!(within(&available, 39) && !within(&available, 40) && !within(&[], 1));
    }

    #[test]
    fn a_pair_held_by_either_channel_is_one_pair_and_an_unresolved_side_none() {
        let none = TagHolderRow::NONE;
        let keys = [key(62, 344, MilitaryAidRow::TALLIES), key(10, 20, MilitaryAidRow::VOLUNTEERS), key(10, 20, MilitaryAidRow::TALLIES), key(10, none, MilitaryAidRow::VOLUNTEERS), key(none, none, MilitaryAidRow::VOLUNTEERS)];
        assert_eq!(pairs(&keys), [(10, 20), (62, 344)], "a pair with only a tally row is a pair, as main listed its link at zero divisions");
    }
}

fn summary_source() -> io::Result<replay_columnar::summary::Source> {
    let series = dashboard_declare::series("military-aid").ok_or_else(|| invalid("Military aid is not declared"))?;
    let measure = series.measure("divisions").ok_or_else(|| invalid("Military aid has no divisions"))?;
    dashboard_declare::source(series.table, measure)
}

