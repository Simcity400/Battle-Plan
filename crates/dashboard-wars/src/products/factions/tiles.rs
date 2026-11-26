use super::{EverIndex, GROUP_LINE, measure_of, merged, within};
use dashboard_base::{
    Source,
    products::{Country, LoadContext, validate_country},
    span::HourSpan,
};
use dashboard_query::{
    Query,
    chart::{ChartBucket, ChartRun},
    series::{
        Ask, Line, MissingSpan,
        ranked::{GroupMembership, GroupSelection},
    },
    tile::{self, BINS, SeriesTileReply, TileLine},
};
use dashboard_text::metadata::check_cancel;
use dashboard_world::products::{group_members, groups::{self, MembershipRun}};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    io,
    sync::atomic::AtomicBool,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub const SERIES: &str = "faction-group-lines";

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, what.into())
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactionGroupTileFamily {
    pub kind: String,
    pub source_kind: String,
    pub measure: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub membership: Option<GroupMembership>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub group: Option<GroupSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub members: Option<Vec<Country>>,
}

pub fn tile_family_walk() -> dashboard_query::fields::Walk {
    dashboard_query::fields::walk::<FactionGroupTileFamily>()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TileAsk {
    branch_id: String,
    #[expect(dead_code, reason = "read as the faction group tile family")]
    family: Value,
    bin_hours: i64,
    index: i64,
    #[serde(default)]
    countries: Vec<Country>,
    within: Option<Vec<(Country, Vec<HourSpan>)>>,
}

fn push_missing(out: &mut Vec<MissingSpan>, first_hour: i32, last_hour: i32, reason: &str) {
    if first_hour > last_hour {
        return;
    }
    if let Some(prev) = out.last_mut()
        && prev.reason == reason
        && prev.last_hour.saturating_add(1) >= first_hour
    {
        prev.last_hour = prev.last_hour.max(last_hour);
        return;
    }
    out.push(MissingSpan { first_hour, last_hour, reason: reason.to_owned() });
}

struct Member {
    spans: Vec<(i32, i32)>,
    runs: Vec<ChartRun>,
    missing: Vec<MissingSpan>,
    changes: Vec<i32>,
}

impl Member {
    fn active(&self, hour: i32) -> bool {
        self.spans.iter().any(|&(a, b)| a <= hour && hour <= b)
    }

    fn value(&self, hour: i32) -> Option<f64> {
        let at = self.runs.partition_point(|run| run.last_hour < hour);
        self.runs.get(at).filter(|run| run.first_hour <= hour).map(|run| run.value)
    }
}

fn covering(spans: &[MissingSpan], hour: i32) -> Option<&MissingSpan> {
    spans.iter().find(|gap| gap.first_hour <= hour && hour <= gap.last_hour)
}

fn summed_runs(members: &[Member], blocked: &[MissingSpan], from: i32, to: i32) -> Vec<(i32, i32, Option<f64>, bool)> {
    let mut edges: Vec<i32> = vec![from, to.saturating_add(1)];
    for member in members {
        edges.extend(&member.changes);
        edges.extend(member.spans.iter().flat_map(|&(a, b)| [a, b.saturating_add(1)]));
        edges.extend(member.runs.iter().flat_map(|run| [run.first_hour, run.last_hour.saturating_add(1)]));
        edges.extend(member.missing.iter().flat_map(|gap| [gap.first_hour, gap.last_hour.saturating_add(1)]));
    }
    edges.extend(blocked.iter().flat_map(|gap| [gap.first_hour, gap.last_hour.saturating_add(1)]));
    edges.retain(|&edge| from <= edge && edge <= to.saturating_add(1));
    edges.sort_unstable();
    edges.dedup();
    let mut out: Vec<(i32, i32, Option<f64>, bool)> = Vec::new();
    for pair in edges.windows(2) {
        let (lo, hi) = (pair[0], pair[1] - 1);
        let active: Vec<&Member> = members.iter().filter(|member| member.active(lo)).collect();
        let value = if active.is_empty() || covering(blocked, lo).is_some() {
            None
        } else {
            active.iter().try_fold(0.0, |sum, member| member.value(lo).map(|value| sum + value))
        };
        let opens = active.iter().any(|member| member.changes.contains(&lo));
        match out.last_mut() {
            Some(held) if !opens && value.is_some() && held.2 == value && held.1.saturating_add(1) == lo => held.1 = hi,
            _ => out.push((lo, hi, value, opens)),
        }
    }
    out
}

fn unread_reason(line: &Line<'_, '_>, drawn: &[(Country, Vec<(i32, i32)>)], members: &[Member], blocked: &[MissingSpan], hour: i32) -> Option<String> {
    if let Some(gap) = covering(blocked, hour) {
        return Some(gap.reason.clone());
    }
    let (country, member) = drawn.iter().map(|(country, _)| country).zip(members).find(|(_, member)| member.active(hour) && member.value(hour).is_none())?;
    Some(
        covering(&member.missing, hour)
            .map(|gap| gap.reason.clone())
            .or_else(|| line.missing_at(country, hour))
            .unwrap_or_else(|| line.query().holders.say(country, hour, |name| format!("{name}'s {} was not recorded at this hour", line.measure_name()))),
    )
}

fn bin_total(total: &[(i32, i32, Option<f64>, bool)], first: i32, clip: i32, bin_hours: i64) -> Vec<Option<ChartBucket>> {
    let mut buckets: Vec<Option<ChartBucket>> = vec![None; BINS];
    let valued_at = |hour: i32| total.iter().any(|&(lo, hi, value, _)| lo <= hour && hour <= hi && value.is_some());
    for (slot, bucket) in buckets.iter_mut().enumerate() {
        let lo = i64::from(first) + slot as i64 * bin_hours;
        let hi = (lo + bin_hours - 1).min(i64::from(clip));
        if lo > hi {
            continue;
        }
        let mut made: Option<ChartBucket> = None;
        let mut gap = false;
        for &(run_lo, run_hi, value, opens) in total {
            let (a, b) = (i64::from(run_lo).max(lo), i64::from(run_hi).min(hi));
            let opens = opens && a == i64::from(run_lo);
            if a > b {
                continue;
            }
            let Some(value) = value else {
                gap |= made.is_some();
                continue;
            };
            let (a, b) = (a as i32, b as i32);
            made = Some(match made {
                None => ChartBucket {
                    absolute_total: 0,
                    first_hour: a,
                    last_hour: b,
                    first_value: value,
                    last_value: value,
                    last_value_hour: b,
                    min_hour: a,
                    min_value: value,
                    max_hour: a,
                    max_value: value,
                    first_break: opens || !valued_at(a - 1),
                    has_break: false,
                },
                Some(held) => ChartBucket {
                    last_hour: b,
                    last_value: value,
                    last_value_hour: b,
                    min_hour: if value < held.min_value { a } else { held.min_hour },
                    min_value: held.min_value.min(value),
                    max_hour: if value > held.max_value { a } else { held.max_hour },
                    max_value: held.max_value.max(value),
                    has_break: held.has_break || gap || opens,
                    ..held
                },
            });
            gap = false;
        }
        *bucket = made;
    }
    buckets
}

fn runs_from_buckets(buckets: &[Option<ChartBucket>]) -> Vec<ChartRun> {
    let mut runs: Vec<ChartRun> = Vec::new();
    for bucket in buckets.iter().flatten() {
        let value = bucket.last_value;
        match runs.last_mut() {
            Some(prev)
                if !bucket.first_break
                    && prev.last_hour.saturating_add(1) == bucket.first_hour
                    && prev.value == value =>
            {
                prev.last_hour = bucket.last_hour;
            }
            _ => runs.push(ChartRun {
                first_hour: bucket.first_hour,
                last_hour: bucket.last_hour,
                value,
                break_before: bucket.first_break,
            }),
        }
    }
    runs
}

struct SummedLine {
    tile: TileLine,
    buckets: Vec<Option<ChartBucket>>,
}

fn empty_line(country: Country, missing: Vec<MissingSpan>) -> SummedLine {
    SummedLine {
        tile: TileLine {
            country,
            part: None,
            missing,
            partial: Vec::new(),
            parts: Vec::new(),
            marks: None,
        },
        buckets: vec![None; BINS],
    }
}

fn sum_member_tiles(
    line: &Line<'_, '_>,
    id: String,
    drawn: &[(Country, Vec<(i32, i32)>)],
    unread: &[MissingSpan],
    first: i32,
    last: i32,
    clip: i32,
    bin_hours: i64,
) -> io::Result<SummedLine> {
    if drawn.is_empty() && unread.is_empty() {
        return Ok(empty_line(Country::Tag(id), Vec::new()));
    }
    let read_from = first.saturating_sub(1);
    let mut members: Vec<Member> = Vec::with_capacity(drawn.len());
    if !drawn.is_empty() {
        let asks: Vec<(Ask, Vec<(i32, i32)>)> = drawn
            .iter()
            .map(|(country, spans)| {
                let clipped = spans.iter().filter_map(|&(a, b)| (a.max(read_from) <= b.min(clip)).then(|| (a.max(read_from), b.min(clip)))).collect();
                (Ask::whole(country), clipped)
            })
            .collect();
        let runs = line.hourly_runs_for_tile(&asks, read_from, clip)?;
        for ((ask, spans), (runs, parts)) in asks.into_iter().zip(runs) {
            let missing = within(&line.missing_for_tile(ask.country.clone(), read_from, clip.min(last), &parts)?, &spans);
            let held = line.query().holders.within(&ask.country, read_from, clip);
            let changes = held.windows(2).filter(|pair| pair[0].last.saturating_add(1) == pair[1].first && pair[0].holder != pair[1].holder).map(|pair| pair[1].first).collect();
            members.push(Member { spans, runs, missing, changes });
        }
    }
    let total = summed_runs(&members, unread, read_from, clip);
    let mut missing: Vec<MissingSpan> = Vec::new();
    for &(lo, hi, value, _) in &total {
        if value.is_none() && hi >= first
            && let Some(reason) = unread_reason(line, drawn, &members, unread, lo)
        {
            push_missing(&mut missing, lo.max(first), hi.min(clip), &reason);
        }
    }
    Ok(SummedLine {
        tile: TileLine {
            country: Country::Tag(id),
            part: None,
            missing,
            partial: Vec::new(),
            parts: Vec::new(),
            marks: None,
        },
        buckets: bin_total(&total, first, clip, bin_hours),
    })
}

fn ever_spans(joins: &[(i32, u32)], first: i32, clip: i32) -> Vec<(Country, Vec<(i32, i32)>)> {
    let mut drawn: Vec<(Country, Vec<(i32, i32)>)> = joins
        .iter()
        .filter_map(|&(start, slot)| {
            let lo = start.max(first);
            (lo <= clip).then(|| (Country::Slot(slot), vec![(lo, clip)]))
        })
        .collect();
    drawn.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    drawn
}

fn hour_spans_from_runs(runs: &[MembershipRun], first: i32, clip: i32) -> Vec<(Country, Vec<(i32, i32)>)> {
    let mut by_tag: BTreeMap<&str, Vec<(i32, i32)>> = BTreeMap::new();
    for run in runs {
        by_tag.entry(run.tag.as_str()).or_default().push((run.first_hour, run.last_hour));
    }
    by_tag
        .into_iter()
        .filter_map(|(tag, spans)| {
            let clipped = merged(spans, first, clip);
            (!clipped.is_empty()).then(|| (Country::Tag(tag.to_owned()), clipped))
        })
        .collect()
}

fn ever_unread(spans: &[HourSpan], first: i32, clip: i32, reason: &str) -> Vec<MissingSpan> {
    let Some(earliest) = spans.iter().filter(|span| span.first_hour <= clip).map(|span| span.first_hour).min() else {
        return Vec::new();
    };
    let lo = earliest.max(first);
    if lo <= clip {
        vec![MissingSpan { first_hour: lo, last_hour: clip, reason: reason.to_owned() }]
    } else {
        Vec::new()
    }
}

pub fn tile(
    source: &Source,
    query: &Query<'_>,
    params: &Value,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<SeriesTileReply> {
    let family_value = params.get("family").ok_or_else(|| invalid("Missing family"))?;
    let family: FactionGroupTileFamily = serde_json::from_value(family_value.clone())
        .map_err(|error| invalid(format!("Invalid faction group tile family: {error}")))?;
    if family.kind != SERIES {
        return Err(invalid(format!("Expected {SERIES} family")));
    }
    let (series, measure) = measure_of(&family.source_kind, &family.measure).map_err(invalid)?;
    let ask: TileAsk = serde_json::from_value(params.clone())
        .map_err(|error| invalid(format!("Invalid faction group tile request: {error}")))?;
    for country in &ask.countries {
        validate_country(country).map_err(invalid)?;
    }
    if let Some(members) = &family.members {
        for country in members {
            validate_country(country).map_err(invalid)?;
        }
    }
    if family.group.is_none() && family.membership != Some(GroupMembership::Ever) && family.members.is_none() {
        return Err(invalid("Faction group tiles require group, membership, or members"));
    }
    if ask.bin_hours < 1 {
        return Err(invalid("Invalid binHours"));
    }
    let (first, last) = tile::hours(ask.bin_hours, ask.index)?;
    let (immutable, filled) = tile::closure(first, last, query.timeline.last);
    let mut built: Vec<SummedLine> = Vec::new();

    if let Some(clip) = filled {
        let line = Line::new(query, series, measure)?;
        let refusal_first = first.max(query.timeline.first).min(clip);
        let read_from = first.saturating_sub(1).max(query.timeline.first);

        if let Some(group) = &family.group {
            let reply = groups::shared(source, query, context, cancelled)?;
            let unread_reason = "The group's members were not read whole here";
            let held = match group.membership {
                GroupMembership::Ever => {
                    let index = EverIndex::shared(source, query.timeline.id, &reply, Some(group))?;
                    match &index.refused {
                        Some(reason) => Err(reason.clone()),
                        None => Ok((
                            ever_spans(index.joins_of(GROUP_LINE), read_from, clip),
                            index
                                .unknown_from
                                .filter(|&from| from <= clip)
                                .map(|from| MissingSpan { first_hour: from.max(read_from), last_hour: clip, reason: unread_reason.to_owned() })
                                .into_iter()
                                .collect::<Vec<_>>(),
                        )),
                    }
                }
                GroupMembership::Hour => group_members::runs(&reply, group)?.map(|found| {
                    (
                        hour_spans_from_runs(&found.members, read_from, clip),
                        merged(found.unknown.iter().map(|s| (s.first_hour, s.last_hour)).collect(), read_from, clip)
                            .into_iter()
                            .map(|(first_hour, last_hour)| MissingSpan { first_hour, last_hour, reason: unread_reason.to_owned() })
                            .collect(),
                    )
                }),
            };
            match held {
                Ok((drawn, unread)) => {
                    check_cancel(cancelled)?;
                    built.push(sum_member_tiles(
                        &line,
                        GROUP_LINE.to_owned(),
                        &drawn,
                        &unread,
                        first,
                        last,
                        clip,
                        ask.bin_hours,
                    )?);
                }
                Err(reason) => {
                    built.push(empty_line(
                        Country::Tag(GROUP_LINE.to_owned()),
                        vec![MissingSpan { first_hour: refusal_first, last_hour: clip, reason }],
                    ));
                }
            }
        } else if family.membership == Some(GroupMembership::Ever) {
            let reply = groups::shared(source, query, context, cancelled)?;
            let index = EverIndex::shared(source, query.timeline.id, &reply, None)?;
            let by_id: BTreeMap<&str, _> =
                reply.factions.iter().map(|faction| (faction.id.as_str(), faction)).collect();
            let requested: Vec<String> = if ask.countries.is_empty() {
                reply.factions.iter().map(|faction| faction.id.clone()).collect()
            } else {
                ask.countries.iter().map(|country| country.key().into_owned()).collect()
            };
            for id in requested {
                check_cancel(cancelled)?;
                let Some(faction) = by_id.get(id.as_str()).copied() else {
                    built.push(empty_line(
                        Country::Tag(id.clone()),
                        vec![MissingSpan {
                            first_hour: refusal_first,
                            last_hour: clip,
                            reason: format!("No faction {id} in this history"),
                        }],
                    ));
                    continue;
                };
                let mut all_unknown = reply.unobserved.factions.clone();
                all_unknown.extend(faction.incomplete.iter().cloned());
                let unread = ever_unread(&all_unknown, read_from, clip, "Faction memberships were not read whole here");
                let drawn = if faction.first_hour > clip {
                    Vec::new()
                } else {
                    ever_spans(index.joins_of(&faction.id), read_from, clip)
                };
                built.push(sum_member_tiles(
                    &line,
                    id,
                    &drawn,
                    &unread,
                    first,
                    last,
                    clip,
                    ask.bin_hours,
                )?);
            }
        } else if let Some(members) = &family.members {
            let within_map: BTreeMap<String, Vec<(i32, i32)>> = ask
                .within
                .unwrap_or_default()
                .into_iter()
                .map(|(country, spans)| {
                    (
                        country.key().into_owned(),
                        spans.into_iter().map(|span| (span.first_hour, span.last_hour)).collect(),
                    )
                })
                .collect();
            let drawn: Vec<(Country, Vec<(i32, i32)>)> = members
                .iter()
                .filter_map(|country| {
                    let raw = within_map
                        .get(country.key().as_ref())
                        .cloned()
                        .unwrap_or_else(|| vec![(read_from, clip)]);
                    let spans = merged(raw, read_from, clip);
                    (!spans.is_empty()).then(|| (country.clone(), spans))
                })
                .collect();
            check_cancel(cancelled)?;
            built.push(sum_member_tiles(
                &line,
                GROUP_LINE.to_owned(),
                &drawn,
                &[],
                first,
                last,
                clip,
                ask.bin_hours,
            )?);
        }
    } else if family.group.is_some() || family.members.is_some() {
        built.push(empty_line(Country::Tag(GROUP_LINE.to_owned()), Vec::new()));
    } else if family.membership == Some(GroupMembership::Ever) {
        for country in &ask.countries {
            built.push(empty_line(country.clone(), Vec::new()));
        }
    }

    let scale = 10f64.powi(i32::from(measure.decimals));
    let (bins, raw_values, hour_bytes) = match ask.bin_hours {
        1 => {
            let run_lines: Vec<Vec<ChartRun>> = built.iter().map(|item| runs_from_buckets(&item.buckets)).collect();
            let (bytes, raw) = tile::encode_runs(&run_lines, first, scale);
            (bytes, raw, 0)
        }
        bin_hours => {
            let all_buckets: Vec<Option<ChartBucket>> = built.iter().flat_map(|item| item.buckets.iter().copied()).collect();
            tile::encode(&all_buckets, first, bin_hours, scale)
        }
    };
    let tile_lines: Vec<TileLine> = built.into_iter().map(|item| item.tile).collect();

    Ok(SeriesTileReply {
        branch_id: ask.branch_id,
        measure: measure.name.to_owned(),
        decimals: measure.decimals,
        bin_hours: ask.bin_hours,
        index: ask.index,
        first_hour: first,
        immutable,
        filled,
        raw_values,
        hour_bytes,
        lines: tile_lines,
        bins: tile::base64(&bins),
        ranking: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_base::{
        SourceKey,
        test_fixtures::{CAMPAIGN, records},
    };
    use replay::campaign::Replay;
    use replay_data::rows::{
        CampaignRow, CountryFactoriesRow, CountryLinkRow, CountryRow, FactionMemberRow, FactionRow,
        HumanControllerRow, Recorded, TableId, TableRows,
    };
    use replay_frame::frame::{DecodedFrame, Metrics, TableView};
    use std::sync::Arc;

    fn bytes(reply: &SeriesTileReply) -> Vec<u8> {
        const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let val = |byte: u8| ALPHABET.iter().position(|&c| c == byte).map_or(0, |at| at as u32);
        let mut out = Vec::new();
        for chunk in reply.bins.as_bytes().chunks_exact(4) {
            let word = chunk.iter().fold(0u32, |word, &byte| (word << 6) | val(byte));
            out.extend([(word >> 16) as u8, (word >> 8) as u8, word as u8]);
        }
        out.truncate(out.len() - reply.bins.bytes().rev().take_while(|&byte| byte == b'=').count());
        out
    }

    fn flags(reply: &SeriesTileReply, line: usize) -> Vec<u8> {
        bytes(reply)[line * BINS..(line + 1) * BINS].to_vec()
    }

    fn build_source() -> Source {
        let links = vec![
            CountryLinkRow { id: 9001, canonical_slot: 7 },
            CountryLinkRow { id: 9002, canonical_slot: 9 },
        ];
        let factions = vec![FactionRow { id: 10, name: b"Axis".to_vec(), ..FactionRow::default() }];
        let frames: Vec<DecodedFrame> = (100..=115)
            .map(|hour| {
                let mut tables = std::array::from_fn(|_| TableView::Unavailable { reason: "outside fixture".into() });
                tables[TableId::Campaign.index()] = TableView::Complete(TableRows::Campaign(vec![CampaignRow { id: CAMPAIGN.into() }]));
                tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
                    CountryRow {
                        border_land_doctrine_level: Recorded::Value(None),
                        id: 9001,
                        canonical_slot: 7,
                        tag: b"GER".to_vec(),
                        ..CountryRow::default()
                    },
                    CountryRow {
                        border_land_doctrine_level: Recorded::Value(None),
                        id: 9002,
                        canonical_slot: 9,
                        tag: b"ITA".to_vec(),
                        ..CountryRow::default()
                    },
                ]));
                tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(links.clone()));
                let mut humans = vec![HumanControllerRow { country: 9001, count: 1 }];
                if hour >= 105 {
                    humans.push(HumanControllerRow { country: 9002, count: 1 });
                }
                tables[TableId::HumanControllers.index()] = TableView::Complete(TableRows::HumanControllers(humans));
                tables[TableId::Factions.index()] = TableView::Complete(TableRows::Factions(factions.clone()));
                let mut members = vec![FactionMemberRow { country: 9001, faction: 10, ordinal: 0 }];
                if hour >= 105 {
                    members.push(FactionMemberRow { country: 9002, faction: 10, ordinal: 1 });
                }
                tables[TableId::FactionMembers.index()] = TableView::Complete(TableRows::FactionMembers(members));
                let fac_ger = CountryFactoriesRow { country: 9001, military_total_raw: if hour >= 110 { 0 } else { 4_000_000 }, ..CountryFactoriesRow::default() };
                let fac_ita = CountryFactoriesRow { country: 9002, military_total_raw: if hour >= 110 { 6_000_000 } else { 2_000_000 }, ..CountryFactoriesRow::default() };
                tables[TableId::CountryFactories.index()] = match hour {
                    111 => TableView::Unavailable { reason: "factories unread at 111".into() },
                    112 => TableView::Complete(TableRows::CountryFactories(vec![fac_ita])),
                    113 => TableView::Complete(TableRows::CountryFactories(vec![fac_ger])),
                    _ => TableView::Complete(TableRows::CountryFactories(vec![fac_ger, fac_ita])),
                };
                DecodedFrame { branch: 1, hour, tables: tables.map(Into::into), metrics: Metrics::default() }
            })
            .collect();
        let built = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)])
            .unwrap()
            .into_timelines(&Default::default())
            .unwrap();
        Source::from_replay(
            SourceKey { source_id: "faction-group-tiles-test".into(), revision: "1".into() },
            "synthetic.battleplan".into(),
            Arc::new(built),
            &AtomicBool::new(false),
        )
        .unwrap()
    }

    fn renamed_member_source(member_through: i32) -> Source {
        let links = vec![CountryLinkRow { id: 9001, canonical_slot: 7 }, CountryLinkRow { id: 9003, canonical_slot: 11 }];
        let factions = vec![FactionRow { id: 10, name: b"Axis".to_vec(), ..FactionRow::default() }];
        let frames: Vec<DecodedFrame> = (100..=115)
            .map(|hour| {
                let mut tables = std::array::from_fn(|_| TableView::Unavailable { reason: "outside fixture".into() });
                tables[TableId::Campaign.index()] = TableView::Complete(TableRows::Campaign(vec![CampaignRow { id: CAMPAIGN.into() }]));
                let member_tag: &[u8] = if hour >= 108 { b"DEU" } else { b"GER" };
                let mut countries = vec![CountryRow { border_land_doctrine_level: Recorded::Value(None), id: 9001, canonical_slot: 7, tag: member_tag.to_vec(), ..CountryRow::default() }];
                if hour >= 108 {
                    countries.push(CountryRow { border_land_doctrine_level: Recorded::Value(None), id: 9003, canonical_slot: 11, tag: b"GER".to_vec(), ..CountryRow::default() });
                }
                tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(countries));
                tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(links.clone()));
                tables[TableId::HumanControllers.index()] = TableView::Complete(TableRows::HumanControllers(vec![HumanControllerRow { country: 9001, count: 1 }]));
                tables[TableId::Factions.index()] = TableView::Complete(TableRows::Factions(factions.clone()));
                let members = if hour <= member_through { vec![FactionMemberRow { country: 9001, faction: 10, ordinal: 0 }] } else { Vec::new() };
                tables[TableId::FactionMembers.index()] = TableView::Complete(TableRows::FactionMembers(members));
                tables[TableId::CountryFactories.index()] = TableView::Complete(TableRows::CountryFactories(vec![
                    CountryFactoriesRow { country: 9001, military_total_raw: 4_000_000, ..CountryFactoriesRow::default() },
                    CountryFactoriesRow { country: 9003, military_total_raw: 7_000_000, ..CountryFactoriesRow::default() },
                ]));
                DecodedFrame { branch: 1, hour, tables: tables.map(Into::into), metrics: Metrics::default() }
            })
            .collect();
        let built = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)])
            .unwrap()
            .into_timelines(&Default::default())
            .unwrap();
        Source::from_replay(
            SourceKey { source_id: "faction-renamed-member".into(), revision: "1".into() },
            "synthetic.battleplan".into(),
            Arc::new(built),
            &AtomicBool::new(false),
        )
        .unwrap()
    }

    #[test]
    fn an_ever_member_is_its_recorded_slot_so_a_later_holder_of_its_old_tag_never_enters_the_total() {
        let src = renamed_member_source(115);
        let query = dashboard_base::queries::query(&src, 1).unwrap();
        let load = LoadContext { content: None };
        let cancelled = AtomicBool::new(false);
        let at_110 = load_at(
            &src,
            serde_json::json!({ "branchId": "1", "hour": 110, "kind": "country-factories", "measure": "free7", "membership": "ever" }),
            &load,
            &cancelled,
        );
        assert_eq!(at_110["rows"][0]["total"], 40, "the member slot renamed to DEU counts; the new holder of GER does not");
        let runs = vec![
            MembershipRun { tag: "GER".to_owned(), first_hour: 100, last_hour: 107 },
            MembershipRun { tag: "DEU".to_owned(), first_hour: 108, last_hour: 115 },
        ];
        let drawn = ever_spans(&super::super::slot_joins(&runs, &query.holders), 0, 115);
        assert_eq!(drawn, vec![(Country::Slot(7), vec![(100, 115)])], "membership belongs to the slot, never to whoever holds GER later");
        let series = dashboard_declare::series("country-factories").unwrap();
        let reader = Line::new(&query, series, series.measure("free7").unwrap()).unwrap();
        let summed = sum_member_tiles(&reader, "10".to_owned(), &drawn, &[], 0, 255, 115, 1).unwrap();
        assert_eq!((summed.buckets[105].map(|b| b.last_value), summed.buckets[112].map(|b| b.last_value)), (Some(40.0), Some(40.0)), "the line stays the member's 40, never 110");

        let explicit = tile(
            &src,
            &query,
            &serde_json::json!({ "branchId": "1", "family": { "kind": SERIES, "sourceKind": "country-factories", "measure": "free7", "members": ["GER"] }, "binHours": 1, "index": 0, "countries": [GROUP_LINE] }),
            &load,
            &cancelled,
        )
        .unwrap();
        assert_eq!(flags(&explicit, 0)[108] & tile::FIRST_BREAK, tile::FIRST_BREAK, "GER passing to another country at 108 starts a new stroke there");

        let left = renamed_member_source(105);
        let group = serde_json::json!({ "type": "faction", "id": "10", "membership": "ever" });
        let chosen = load_at(&left, serde_json::json!({ "branchId": "1", "hour": 110, "kind": "country-factories", "measure": "free7", "group": group }), &load, &cancelled);
        assert_eq!(chosen["rows"][0]["total"], 40, "a chosen group counted ever sums its member slot, never the later holder of the tag it left with");
        for asked in [
            serde_json::json!({ "branchId": "1", "kind": "country-factories", "measure": "free7", "membership": "ever" }),
            serde_json::json!({ "branchId": "1", "kind": "country-factories", "measure": "free7", "group": group }),
        ] {
            let panel = dashboard_base::products::at_hour::exact_panel::<super::super::FactionTotalsAt>(&left, &load, &asked).unwrap();
            let exact = panel.exact().unwrap();
            for hour in 100..=115 {
                let played = (exact.advance)(hour, hour == 100, &cancelled, None);
                let played = played.value.as_ref().as_ref().unwrap();
                let cold = (exact.cold)(hour, &cancelled).unwrap();
                assert_eq!(played["rows"], cold["rows"], "playback reads the member slot's own row at {hour} for {asked}");
                assert_eq!(cold["rows"][0]["total"], 40, "the member that left is still counted, from its own slot, at {hour} for {asked}");
            }
        }
    }

    #[test]
    fn a_member_slot_with_no_tag_while_it_was_a_member_is_still_ever_a_member() {
        let links = vec![CountryLinkRow { id: 9001, canonical_slot: 7 }, CountryLinkRow { id: 9002, canonical_slot: 9 }];
        let frames: Vec<DecodedFrame> = (100..=115)
            .map(|hour| {
                let mut tables = std::array::from_fn(|_| TableView::Unavailable { reason: "outside fixture".into() });
                tables[TableId::Campaign.index()] = TableView::Complete(TableRows::Campaign(vec![CampaignRow { id: CAMPAIGN.into() }]));
                let mut countries = vec![CountryRow { border_land_doctrine_level: Recorded::Value(None), id: 9001, canonical_slot: 7, tag: b"GER".to_vec(), ..CountryRow::default() }];
                if hour >= 108 {
                    countries.push(CountryRow { border_land_doctrine_level: Recorded::Value(None), id: 9002, canonical_slot: 9, tag: b"ITA".to_vec(), ..CountryRow::default() });
                }
                tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(countries));
                tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(links.clone()));
                tables[TableId::HumanControllers.index()] = TableView::Complete(TableRows::HumanControllers(Vec::new()));
                tables[TableId::Factions.index()] = TableView::Complete(TableRows::Factions(vec![FactionRow { id: 10, name: b"Axis".to_vec(), ..FactionRow::default() }]));
                let mut members = vec![FactionMemberRow { country: 9001, faction: 10, ordinal: 0 }];
                if hour <= 104 {
                    members.push(FactionMemberRow { country: 9002, faction: 10, ordinal: 1 });
                }
                tables[TableId::FactionMembers.index()] = TableView::Complete(TableRows::FactionMembers(members));
                tables[TableId::CountryFactories.index()] = TableView::Complete(TableRows::CountryFactories(vec![
                    CountryFactoriesRow { country: 9001, military_total_raw: 4_000_000, ..CountryFactoriesRow::default() },
                    CountryFactoriesRow { country: 9002, military_total_raw: 2_000_000, ..CountryFactoriesRow::default() },
                ]));
                DecodedFrame { branch: 1, hour, tables: tables.map(Into::into), metrics: Metrics::default() }
            })
            .collect();
        let built = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
        let src = Source::from_replay(SourceKey { source_id: "faction-untagged-member".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(built), &AtomicBool::new(false)).unwrap();
        let (load, cancelled) = (LoadContext { content: None }, AtomicBool::new(false));
        let at = load_at(&src, serde_json::json!({ "branchId": "1", "hour": 110, "kind": "country-factories", "measure": "free7", "membership": "ever" }), &load, &cancelled);
        assert_eq!(at["rows"][0]["total"], 60, "slot 9 was a member at 100 to 104 though it held no tag then: {at}");
        let query = dashboard_base::queries::query(&src, 1).unwrap();
        for family in [
            serde_json::json!({ "kind": SERIES, "sourceKind": "country-factories", "measure": "free7", "membership": "ever" }),
            serde_json::json!({ "kind": SERIES, "sourceKind": "country-factories", "measure": "free7", "group": { "type": "faction", "id": "10", "membership": "ever" } }),
        ] {
            let line = if family.get("group").is_some() { GROUP_LINE } else { "10" };
            let reply = tile(&src, &query, &serde_json::json!({ "branchId": "1", "family": family, "binHours": 1, "index": 0, "countries": [line] }), &load, &cancelled).unwrap();
            let drawn = tile::decode_runs(&bytes(&reply), 1, reply.first_hour, 10f64.powi(i32::from(reply.decimals)), reply.raw_values).unwrap().remove(0);
            let at_110 = drawn.iter().find(|run| run.0 <= 110 && 110 <= run.1).map(|run| run.2);
            assert_eq!(at_110, Some(60.0), "the tile draws the untagged member too, for {family}: {drawn:?}");
        }
    }

    #[test]
    fn a_faction_whose_only_member_is_unresolved_refuses_a_top_rather_than_vanishing() {
        let links = vec![CountryLinkRow { id: 9001, canonical_slot: 7 }];
        let frames: Vec<DecodedFrame> = (100..=110)
            .map(|hour| {
                let mut tables = std::array::from_fn(|_| TableView::Unavailable { reason: "outside fixture".into() });
                tables[TableId::Campaign.index()] = TableView::Complete(TableRows::Campaign(vec![CampaignRow { id: CAMPAIGN.into() }]));
                tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![CountryRow { border_land_doctrine_level: Recorded::Value(None), id: 9001, canonical_slot: 7, tag: b"GER".to_vec(), ..CountryRow::default() }]));
                if hour == 107 {
                    tables[TableId::Countries.index()] = TableView::Unavailable { reason: "country roster unread".into() };
                }
                tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(if hour == 108 { Vec::new() } else { links.clone() }));
                tables[TableId::HumanControllers.index()] = TableView::Complete(TableRows::HumanControllers(Vec::new()));
                tables[TableId::Factions.index()] = TableView::Complete(TableRows::Factions(vec![
                    FactionRow { id: 10, name: b"Axis".to_vec(), ..FactionRow::default() },
                    FactionRow { id: 20, name: b"Allies".to_vec(), ..FactionRow::default() },
                ]));
                tables[TableId::FactionMembers.index()] = TableView::Complete(TableRows::FactionMembers(vec![
                    FactionMemberRow { country: 9001, faction: 10, ordinal: 0 },
                    FactionMemberRow { country: 9009, faction: 20, ordinal: 0 },
                ]));
                tables[TableId::CountryFactories.index()] = TableView::Complete(TableRows::CountryFactories(vec![CountryFactoriesRow { country: 9001, military_total_raw: 4_000_000, ..CountryFactoriesRow::default() }]));
                DecodedFrame { branch: 1, hour, tables: tables.map(Into::into), metrics: Metrics::default() }
            })
            .collect();
        let built = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
        let src = Source::from_replay(SourceKey { source_id: "faction-unresolved".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(built), &AtomicBool::new(false)).unwrap();
        let (load, cancelled) = (LoadContext { content: None }, AtomicBool::new(false));
        let at = load_at(&src, serde_json::json!({ "branchId": "1", "hour": 105, "kind": "country-factories", "measure": "free7", "top": 1, "membership": "ever" }), &load, &cancelled);
        assert!(at["missing"].is_string(), "the unresolved faction's total is unknown, so no top is proved: {at}");
        let now = load_at(&src, serde_json::json!({ "branchId": "1", "hour": 105, "kind": "country-factories", "measure": "free7", "top": 1 }), &load, &cancelled);
        assert!(now["missing"].is_string(), "counted by the hour's members too: {now}");
        let chosen = load_at(&src, serde_json::json!({ "branchId": "1", "hour": 107, "kind": "country-factories", "measure": "free7", "members": ["GER"] }), &load, &cancelled);
        let query = dashboard_base::queries::query(&src, 1).unwrap();
        let reason = query.holders.not_known(&Country::from("GER"), 107).unwrap();
        assert!(chosen["rows"][0]["total"].is_null());
        assert_eq!(chosen["rows"][0]["missing"], reason);
        let unlinked = load_at(&src, serde_json::json!({ "branchId": "1", "hour": 108, "kind": "country-factories", "measure": "free7", "members": ["GER"] }), &load, &cancelled);
        assert_eq!(unlinked["rows"][0]["missing"], format!("Who holds GER is not known: {}", dashboard_query::identity::TAG_UNLINKED));
        let slot = load_at(&src, serde_json::json!({ "branchId": "1", "hour": 108, "kind": "country-factories", "measure": "free7", "members": [7] }), &load, &cancelled);
        assert_eq!(slot["rows"][0]["missing"], format!("Who holds slot 7 is not known: {}", dashboard_query::identity::TAG_UNLINKED));
    }

    fn load_at(source: &Source, params: Value, load: &LoadContext, cancelled: &AtomicBool) -> Value {
        let text = (crate::registrations::FACTIONS_FACTIONTOTALSAT.load)(source, "test", params, load, cancelled).unwrap();
        let envelope: Value = serde_json::from_str(&text).unwrap();
        envelope["data"].clone()
    }

    #[test]
    fn faction_group_tiles_and_ever_head_mark_unread_hour_missing_and_keep_valid_through() {
        let src = build_source();
        let query = dashboard_base::queries::query(&src, 1).unwrap();
        let load = LoadContext { content: None };
        let cancelled = AtomicBool::new(false);

        let reply = tile(
            &src,
            &query,
            &serde_json::json!({
                "branchId": "1",
                "family": {
                    "kind": SERIES,
                    "sourceKind": "country-factories",
                    "measure": "free7",
                    "group": { "type": "humans", "membership": "ever" }
                },
                "binHours": 1,
                "index": 0,
                "countries": ["group"]
            }),
            &load,
            &cancelled,
        )
        .unwrap();
        assert_eq!(reply.lines.len(), 1);
        let line = &reply.lines[0];
        assert_eq!(line.country, Country::Tag("group".into()));
        assert!(!reply.bins.is_empty());
        assert!(line.missing.iter().any(|gap| gap.first_hour <= 111 && 111 <= gap.last_hour));

        let series = dashboard_declare::series("country-factories").unwrap();
        let measure = series.measure("free7").unwrap();
        let reader = Line::new(&query, series, measure).unwrap();
        let drawn = vec![
            (Country::Tag("GER".into()), vec![(100, 115)]),
            (Country::Tag("ITA".into()), vec![(105, 115)]),
        ];
        let summed = sum_member_tiles(&reader, GROUP_LINE.to_owned(), &drawn, &[], 0, 255, 115, 1).unwrap();
        assert_eq!(summed.buckets[100].map(|b| b.last_value), Some(40.0));
        assert_eq!(summed.buckets[105].map(|b| b.last_value), Some(60.0));
        assert_eq!(summed.buckets[110].map(|b| b.last_value), Some(60.0));
        assert_eq!(summed.buckets[111], None, "unread hour 111 must never show stale 60");
        assert_eq!(summed.buckets[113], None, "a member whose value is not recorded makes the total unknown, never the other members' sum");
        assert!(summed.tile.missing.iter().any(|gap| gap.first_hour <= 113 && 113 <= gap.last_hour), "and says why");
        let reasons_at = |hour: i32| summed.tile.missing.iter().filter(|gap| gap.first_hour <= hour && hour <= gap.last_hour).map(|gap| gap.reason.as_str()).collect::<Vec<_>>();
        assert!(!reasons_at(112).is_empty() && reasons_at(112).iter().all(|reason| reason.contains("GER")), "hour 112 is missing for GER: {:?}", summed.tile.missing);
        assert!(!reasons_at(113).is_empty() && reasons_at(113).iter().all(|reason| reason.contains("ITA")), "and hour 113 for ITA, never GER's reason carried on: {:?}", summed.tile.missing);
        assert_eq!((summed.buckets[100].unwrap().first_break, summed.buckets[101].unwrap().first_break, summed.buckets[114].unwrap().first_break), (true, false, true), "a stroke breaks where a value is missing, not elsewhere");

        let coarse = sum_member_tiles(&reader, GROUP_LINE.to_owned(), &drawn, &[], 0, 1535, 115, 6).unwrap();
        let swap = coarse.buckets[18].expect("hours 108 to 113 keep their recorded totals around the unread hours");
        assert_eq!((swap.min_value, swap.max_value), (60.0, 60.0), "two members trading 40 for 20 inside a bin is a total that stayed 60, never a dip or a spike");
        assert!(coarse.buckets[19].unwrap().first_break, "the unread hours closing that bin break the stroke into the next");
        let later = sum_member_tiles(&reader, GROUP_LINE.to_owned(), &drawn, &[], 101, 356, 115, 1).unwrap();
        assert!(!later.buckets[0].unwrap().first_break, "a tile starting inside a recorded stretch continues the stroke from the hour before it");

        let ever_factions = tile(
            &src,
            &query,
            &serde_json::json!({
                "branchId": "1",
                "family": {
                    "kind": SERIES,
                    "sourceKind": "country-factories",
                    "measure": "free7",
                    "membership": "ever"
                },
                "binHours": 1,
                "index": 0,
                "countries": ["10"]
            }),
            &load,
            &cancelled,
        )
        .unwrap();
        assert_eq!(ever_factions.lines.len(), 1);
        assert!(ever_factions.lines[0].missing.iter().any(|gap| gap.first_hour <= 111 && 111 <= gap.last_hour));

        let at_104 = load_at(
            &src,
            serde_json::json!({ "branchId": "1", "hour": 104, "kind": "country-factories", "measure": "free7", "top": 5, "membership": "ever" }),
            &load,
            &cancelled,
        );
        assert_eq!(at_104["rows"].as_array().unwrap().len(), 1);
        assert_eq!(at_104["rows"][0]["total"], 40);
        let at_110 = load_at(
            &src,
            serde_json::json!({ "branchId": "1", "hour": 110, "kind": "country-factories", "measure": "free7", "top": 5, "membership": "ever" }),
            &load,
            &cancelled,
        );
        assert_eq!(at_110["rows"][0]["total"], 60);
        let at_111 = load_at(
            &src,
            serde_json::json!({ "branchId": "1", "hour": 111, "kind": "country-factories", "measure": "free7", "top": 5, "membership": "ever" }),
            &load,
            &cancelled,
        );
        assert!(at_111["missing"].is_string(), "top N ever-members must refuse when a member is unread at 111");

        let group_111 = load_at(
            &src,
            serde_json::json!({
                "branchId": "1",
                "hour": 111,
                "kind": "country-factories",
                "measure": "free7",
                "group": { "type": "humans", "membership": "ever" }
            }),
            &load,
            &cancelled,
        );
        assert_eq!(group_111["rows"].as_array().unwrap().len(), 1);
        assert!(group_111["rows"][0]["total"].is_null());
        assert!(group_111["rows"][0]["missing"].is_string());
    }

    fn build_late_faction_source(unobserved_factions: Option<(i32, i32)>) -> Source {
        let links = vec![
            CountryLinkRow { id: 9001, canonical_slot: 7 },
            CountryLinkRow { id: 9002, canonical_slot: 9 },
        ];
        let fac_ger = CountryFactoriesRow { country: 9001, military_total_raw: 4_000_000, ..CountryFactoriesRow::default() };
        let fac_ita = CountryFactoriesRow { country: 9002, military_total_raw: 2_000_000, ..CountryFactoriesRow::default() };
        let frames: Vec<DecodedFrame> = (100..=300)
            .map(|hour| {
                let mut tables = std::array::from_fn(|_| TableView::Unavailable { reason: "outside fixture".into() });
                tables[TableId::Campaign.index()] = TableView::Complete(TableRows::Campaign(vec![CampaignRow { id: CAMPAIGN.into() }]));
                tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
                    CountryRow {
                        border_land_doctrine_level: Recorded::Value(None),
                        id: 9001,
                        canonical_slot: 7,
                        tag: b"GER".to_vec(),
                        ..CountryRow::default()
                    },
                    CountryRow {
                        border_land_doctrine_level: Recorded::Value(None),
                        id: 9002,
                        canonical_slot: 9,
                        tag: b"ITA".to_vec(),
                        ..CountryRow::default()
                    },
                ]));
                tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(links.clone()));
                let humans = if hour >= 270 {
                    vec![HumanControllerRow { country: 9002, count: 1 }]
                } else {
                    Vec::new()
                };
                tables[TableId::HumanControllers.index()] = TableView::Complete(TableRows::HumanControllers(humans));
                if unobserved_factions.is_some_and(|(from, to)| from <= hour && hour <= to) {
                    tables[TableId::Factions.index()] = TableView::Unavailable { reason: "factions unread".into() };
                    tables[TableId::FactionMembers.index()] = TableView::Unavailable { reason: "factions unread".into() };
                } else {
                    let mut factions = vec![FactionRow { id: 10, name: b"Axis".to_vec(), ..FactionRow::default() }];
                    let mut members = vec![FactionMemberRow { country: 9001, faction: 10, ordinal: 0 }];
                    if hour >= 270 {
                        factions.push(FactionRow { id: 20, name: b"Allies".to_vec(), ..FactionRow::default() });
                        members.push(FactionMemberRow { country: 9002, faction: 20, ordinal: 0 });
                    }
                    tables[TableId::Factions.index()] = TableView::Complete(TableRows::Factions(factions));
                    tables[TableId::FactionMembers.index()] = TableView::Complete(TableRows::FactionMembers(members));
                }
                tables[TableId::CountryFactories.index()] = TableView::Complete(TableRows::CountryFactories(vec![fac_ger.clone(), fac_ita.clone()]));
                DecodedFrame { branch: 1, hour, tables: tables.map(Into::into), metrics: Metrics::default() }
            })
            .collect();
        let built = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)])
            .unwrap()
            .into_timelines(&Default::default())
            .unwrap();
        Source::from_replay(
            SourceKey { source_id: "late-faction-tiles-test".into(), revision: "1".into() },
            "synthetic.battleplan".into(),
            Arc::new(built),
            &AtomicBool::new(false),
        )
        .unwrap()
    }

    #[test]
    fn ever_and_group_tiles_return_one_line_per_asked_key_in_order_before_founding_or_when_roster_unknown() {
        let load = LoadContext { content: None };
        let cancelled = AtomicBool::new(false);

        let src = build_late_faction_source(None);
        let query = dashboard_base::queries::query(&src, 1).unwrap();

        let early = tile(
            &src,
            &query,
            &serde_json::json!({
                "branchId": "1",
                "family": {
                    "kind": SERIES,
                    "sourceKind": "country-factories",
                    "measure": "free7",
                    "membership": "ever"
                },
                "binHours": 1,
                "index": 0,
                "countries": ["20", "10"]
            }),
            &load,
            &cancelled,
        )
        .unwrap();
        assert_eq!(early.lines.len(), 2, "must emit one line per asked key even before faction 20 was founded");
        assert_eq!(early.lines[0].country, Country::Tag("20".into()), "must keep request order");
        assert_eq!(early.lines[1].country, Country::Tag("10".into()), "must keep request order");
        assert!(early.lines[0].missing.is_empty(), "observed pre-founding tile is empty, not missing");
        assert!(early.lines[1].missing.is_empty());
        assert!(!early.bins.is_empty());

        let unknown_faction = tile(
            &src,
            &query,
            &serde_json::json!({
                "branchId": "1",
                "family": {
                    "kind": SERIES,
                    "sourceKind": "country-factories",
                    "measure": "free7",
                    "membership": "ever"
                },
                "binHours": 1,
                "index": 0,
                "countries": ["10", "999"]
            }),
            &load,
            &cancelled,
        )
        .unwrap();
        assert_eq!(unknown_faction.lines.len(), 2);
        assert_eq!(unknown_faction.lines[1].country, Country::Tag("999".into()));
        assert!(!unknown_faction.lines[1].missing.is_empty(), "unknown asked faction must carry a refusal reason");

        let empty_group = tile(
            &src,
            &query,
            &serde_json::json!({
                "branchId": "1",
                "family": {
                    "kind": SERIES,
                    "sourceKind": "country-factories",
                    "measure": "free7",
                    "group": { "type": "humans", "membership": "hour" }
                },
                "binHours": 1,
                "index": 0,
                "countries": ["group"]
            }),
            &load,
            &cancelled,
        )
        .unwrap();
        assert_eq!(empty_group.lines.len(), 1, "group with no members in tile must still emit one empty group line");
        assert_eq!(empty_group.lines[0].country, Country::Tag(GROUP_LINE.into()));
        assert!(empty_group.lines[0].missing.is_empty());
        assert!(!empty_group.bins.is_empty());

        let unknown_group = tile(
            &src,
            &query,
            &serde_json::json!({
                "branchId": "1",
                "family": {
                    "kind": SERIES,
                    "sourceKind": "country-factories",
                    "measure": "free7",
                    "group": { "type": "faction", "id": "999", "membership": "ever" }
                },
                "binHours": 1,
                "index": 0,
                "countries": ["group"]
            }),
            &load,
            &cancelled,
        )
        .unwrap();
        assert_eq!(unknown_group.lines.len(), 1, "unknown group must still emit one group line with a refusal reason");
        assert_eq!(unknown_group.lines[0].country, Country::Tag(GROUP_LINE.into()));
        assert!(!unknown_group.lines[0].missing.is_empty());

        let past_end = tile(
            &src,
            &query,
            &serde_json::json!({
                "branchId": "1",
                "family": {
                    "kind": SERIES,
                    "sourceKind": "country-factories",
                    "measure": "free7",
                    "membership": "ever"
                },
                "binHours": 1,
                "index": 2,
                "countries": ["20", "10"]
            }),
            &load,
            &cancelled,
        )
        .unwrap();
        assert_eq!(past_end.lines.len(), 2, "unfilled tile past timeline end must still emit one line per asked key");
        assert_eq!(past_end.lines[0].country, Country::Tag("20".into()));
        assert_eq!(past_end.lines[1].country, Country::Tag("10".into()));
        assert!(!past_end.bins.is_empty());

        let unread_src = build_late_faction_source(Some((100, 120)));
        let unread_query = dashboard_base::queries::query(&unread_src, 1).unwrap();
        let groups = dashboard_world::products::groups::shared(&unread_src, &unread_query, &load, &cancelled).unwrap();
        let proof = EverIndex::shared(&unread_src, 1, &groups, None).unwrap();
        assert_eq!(proof.valid_through(120), i32::MAX, "a later founding cannot change the refusal after an unread roster");
        let unread_early = tile(
            &unread_src,
            &unread_query,
            &serde_json::json!({
                "branchId": "1",
                "family": {
                    "kind": SERIES,
                    "sourceKind": "country-factories",
                    "measure": "free7",
                    "membership": "ever"
                },
                "binHours": 1,
                "index": 0,
                "countries": ["10", "20"]
            }),
            &load,
            &cancelled,
        )
        .unwrap();
        assert_eq!(unread_early.lines.len(), 2);
        assert_eq!(unread_early.lines[1].country, Country::Tag("20".into()));
        assert!(!unread_early.lines[1].missing.is_empty(), "when faction rosters are unobserved before founding, emit a refusal reason rather than an empty line");

        let explicit_members = tile(
            &src,
            &query,
            &serde_json::json!({
                "branchId": "1",
                "family": {
                    "kind": SERIES,
                    "sourceKind": "country-factories",
                    "measure": "free7",
                    "members": [7, 9]
                },
                "binHours": 1,
                "index": 0,
                "countries": ["group"]
            }),
            &load,
            &cancelled,
        )
        .unwrap();
        assert_eq!(explicit_members.lines.len(), 1);
        assert_eq!(explicit_members.lines[0].country, Country::Tag(GROUP_LINE.into()));
        assert!(explicit_members.lines[0].missing.is_empty());
        assert!(!explicit_members.bins.is_empty());

        let explicit_past_end = tile(
            &src,
            &query,
            &serde_json::json!({
                "branchId": "1",
                "family": {
                    "kind": SERIES,
                    "sourceKind": "country-factories",
                    "measure": "free7",
                    "members": [7, 9]
                },
                "binHours": 1,
                "index": 2,
                "countries": ["group"]
            }),
            &load,
            &cancelled,
        )
        .unwrap();
        assert_eq!(explicit_past_end.lines.len(), 1);
        assert_eq!(explicit_past_end.lines[0].country, Country::Tag(GROUP_LINE.into()));
        assert!(!explicit_past_end.bins.is_empty());

        let family = |extra: serde_json::Value| {
            let mut family = serde_json::json!({ "kind": SERIES, "sourceKind": "country-factories", "measure": "free7" });
            family.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            family
        };
        for (extra, line) in [
            (serde_json::json!({ "membership": "ever" }), "10"),
            (serde_json::json!({ "group": { "type": "faction", "id": "10", "membership": "hour" } }), GROUP_LINE),
            (serde_json::json!({ "group": { "type": "faction", "id": "10", "membership": "ever" } }), GROUP_LINE),
            (serde_json::json!({ "members": [7] }), GROUP_LINE),
        ] {
            let asked = serde_json::json!({ "branchId": "1", "family": family(extra.clone()), "binHours": 1, "index": 1, "countries": [line] });
            let second = tile(&src, &query, &asked, &load, &cancelled).unwrap();
            let edge = flags(&second, 0)[0];
            assert_eq!((edge & tile::PRESENT, edge & tile::FIRST_BREAK), (tile::PRESENT, 0), "hour 256 continues the stroke recorded through 255 for {extra}");
        }

        let gap_src = build_late_faction_source(Some((150, 160)));
        let gap_query = dashboard_base::queries::query(&gap_src, 1).unwrap();
        let gapped = tile(
            &gap_src,
            &gap_query,
            &serde_json::json!({ "branchId": "1", "family": family(serde_json::json!({ "membership": "ever" })), "binHours": 1, "index": 0, "countries": ["10"] }),
            &load,
            &cancelled,
        )
        .unwrap();
        let drawn = flags(&gapped, 0);
        assert_eq!(drawn[140] & tile::PRESENT, tile::PRESENT);
        assert_eq!(drawn[170] & tile::PRESENT, 0, "after the rosters went unread nobody can say who was ever a member, so no subtotal is drawn");
        assert!(gapped.lines[0].missing.iter().any(|gap| gap.first_hour <= 170 && 170 <= gap.last_hour));
    }
}
