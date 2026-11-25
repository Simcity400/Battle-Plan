use super::{GROUP_LINE, measure_of, merged, within};
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
use dashboard_world::products::{group_members, groups};
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

fn combine_bucket(held: Option<ChartBucket>, bucket: ChartBucket) -> ChartBucket {
    match held {
        None => bucket,
        Some(prev) => {
            let first_hour = prev.first_hour.min(bucket.first_hour);
            let first_value = match prev.first_hour.cmp(&bucket.first_hour) {
                std::cmp::Ordering::Less => prev.first_value,
                std::cmp::Ordering::Greater => bucket.first_value,
                std::cmp::Ordering::Equal => prev.first_value + bucket.first_value,
            };
            let last_hour = prev.last_hour.max(bucket.last_hour);
            let last_value = match prev.last_hour.cmp(&bucket.last_hour) {
                std::cmp::Ordering::Greater => prev.last_value,
                std::cmp::Ordering::Less => bucket.last_value,
                std::cmp::Ordering::Equal => prev.last_value + bucket.last_value,
            };
            let partial = prev.first_hour != bucket.first_hour || prev.last_hour != bucket.last_hour;
            let min_value = if partial {
                prev.min_value.min(bucket.min_value).min(prev.min_value + bucket.min_value)
            } else {
                prev.min_value + bucket.min_value
            };
            let max_value = if partial {
                prev.max_value.max(bucket.max_value).max(prev.max_value + bucket.max_value)
            } else {
                prev.max_value + bucket.max_value
            };
            ChartBucket {
                absolute_total: 0,
                first_hour,
                last_hour,
                first_value,
                last_value,
                last_value_hour: prev.last_value_hour.max(bucket.last_value_hour),
                min_hour: prev.min_hour.min(bucket.min_hour),
                min_value,
                max_hour: prev.max_hour.max(bucket.max_hour),
                max_value,
                first_break: prev.first_break && bucket.first_break,
                has_break: prev.has_break || bucket.has_break || partial,
            }
        }
    }
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
    let mut buckets: Vec<Option<ChartBucket>> = vec![None; BINS];
    let mut raw_missing: Vec<MissingSpan> = unread.to_vec();
    if !drawn.is_empty() {
        let asks: Vec<(Ask, Vec<(i32, i32)>)> = drawn
            .iter()
            .map(|(country, spans)| (Ask::whole(country), spans.clone()))
            .collect();
        let member_tiles = line.tile_buckets(&asks, first, last, bin_hours)?;
        for ((_, spans), (member_buckets, member_missing)) in drawn.iter().zip(member_tiles) {
            raw_missing.extend(within(&member_missing, spans));
            for (slot, bucket) in member_buckets.into_iter().enumerate().take(BINS) {
                let Some(bucket) = bucket else { continue };
                buckets[slot] = Some(combine_bucket(buckets[slot].take(), bucket));
            }
        }
    }
    raw_missing.sort_unstable_by(|a, b| (a.first_hour, a.last_hour, &a.reason).cmp(&(b.first_hour, b.last_hour, &b.reason)));
    let mut missing: Vec<MissingSpan> = Vec::new();
    for gap in raw_missing {
        let lo = gap.first_hour.max(first);
        let hi = gap.last_hour.min(clip);
        push_missing(&mut missing, lo, hi, &gap.reason);
    }
    let start_bin = i64::from(first);
    for (slot, bucket) in buckets.iter_mut().enumerate() {
        let lo = start_bin + slot as i64 * bin_hours;
        let hi = lo + bin_hours - 1;
        if lo > i64::from(clip) || missing.iter().any(|gap| i64::from(gap.first_hour) <= hi && lo <= i64::from(gap.last_hour)) {
            *bucket = None;
        }
    }
    for slot in 0..buckets.len() {
        let joined = slot > 0 && buckets[slot - 1].is_some();
        if let Some(bucket) = &mut buckets[slot] {
            bucket.first_break = !joined;
        }
    }
    Ok(SummedLine {
        tile: TileLine {
            country: Country::Tag(id),
            part: None,
            missing,
            parts: Vec::new(),
            marks: None,
        },
        buckets,
    })
}

fn ever_spans_from_runs(runs: &[(String, i32, i32)], first: i32, clip: i32) -> Vec<(Country, Vec<(i32, i32)>)> {
    let mut earliest: BTreeMap<&str, i32> = BTreeMap::new();
    for (tag, run_first, _) in runs {
        if *run_first <= clip {
            earliest
                .entry(tag.as_str())
                .and_modify(|prev| *prev = (*prev).min(*run_first))
                .or_insert(*run_first);
        }
    }
    earliest
        .into_iter()
        .filter_map(|(tag, start)| {
            let lo = start.max(first);
            (lo <= clip).then(|| (Country::Tag(tag.to_owned()), vec![(lo, clip)]))
        })
        .collect()
}

fn hour_spans_from_runs(runs: &[(String, i32, i32)], first: i32, clip: i32) -> Vec<(Country, Vec<(i32, i32)>)> {
    let mut by_tag: BTreeMap<&str, Vec<(i32, i32)>> = BTreeMap::new();
    for (tag, run_first, run_last) in runs {
        by_tag.entry(tag.as_str()).or_default().push((*run_first, *run_last));
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
    if ask.bin_hours < 1 {
        return Err(invalid("Invalid binHours"));
    }
    let (first, last) = tile::hours(ask.bin_hours, ask.index)?;
    let (immutable, filled) = tile::closure(first, last, query.timeline.last);
    let mut built: Vec<SummedLine> = Vec::new();

    if let Some(clip) = filled {
        let line = Line::new(query, series, measure)?;
        let refusal_first = first.max(query.timeline.first).min(clip);

        if let Some(group) = &family.group {
            let reply = groups::shared(source, query, context, cancelled)?;
            match group_members::runs(&reply, group)? {
                Ok(found) => {
                    let member_runs: Vec<(String, i32, i32)> = found
                        .members
                        .iter()
                        .map(|run| (run.tag.clone(), run.first_hour, run.last_hour))
                        .collect();
                    let (drawn, unread) = match group.membership {
                        GroupMembership::Hour => (
                            hour_spans_from_runs(&member_runs, first, clip),
                            merged(found.unknown.iter().map(|s| (s.first_hour, s.last_hour)).collect(), first, clip)
                                .into_iter()
                                .map(|(first_hour, last_hour)| MissingSpan {
                                    first_hour,
                                    last_hour,
                                    reason: "The group's members were not read whole here".to_owned(),
                                })
                                .collect(),
                        ),
                        GroupMembership::Ever => (
                            ever_spans_from_runs(&member_runs, first, clip),
                            ever_unread(&found.unknown, first, clip, "The group's members were not read whole here"),
                        ),
                    };
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
                let unread = ever_unread(&all_unknown, first, clip, "Faction memberships were not read whole here");
                let drawn = if faction.first_hour > clip {
                    Vec::new()
                } else {
                    let member_runs: Vec<(String, i32, i32)> = faction
                        .members
                        .iter()
                        .map(|run| (run.tag.clone(), run.first_hour, run.last_hour))
                        .collect();
                    ever_spans_from_runs(&member_runs, first, clip)
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
        } else if !ask.countries.is_empty() {
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
            let drawn: Vec<(Country, Vec<(i32, i32)>)> = ask
                .countries
                .iter()
                .filter_map(|country| {
                    let raw = within_map
                        .get(country.key().as_ref())
                        .cloned()
                        .unwrap_or_else(|| vec![(first, clip)]);
                    let spans = merged(raw, first, clip);
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
    } else if family.group.is_some() {
        built.push(empty_line(Country::Tag(GROUP_LINE.to_owned()), Vec::new()));
    } else if family.membership == Some(GroupMembership::Ever) {
        for country in &ask.countries {
            built.push(empty_line(country.clone(), Vec::new()));
        }
    } else if !ask.countries.is_empty() {
        built.push(empty_line(Country::Tag(GROUP_LINE.to_owned()), Vec::new()));
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

    fn build_source() -> Source {
        let links = vec![
            CountryLinkRow { id: 9001, canonical_slot: 7 },
            CountryLinkRow { id: 9002, canonical_slot: 9 },
        ];
        let factions = vec![FactionRow { id: 10, name: b"Axis".to_vec(), ..FactionRow::default() }];
        let fac_ger = CountryFactoriesRow { country: 9001, military_total_raw: 4_000_000, ..CountryFactoriesRow::default() };
        let fac_ita = CountryFactoriesRow { country: 9002, military_total_raw: 2_000_000, ..CountryFactoriesRow::default() };
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
                tables[TableId::CountryFactories.index()] = if hour == 111 {
                    TableView::Unavailable { reason: "factories unread at 111".into() }
                } else {
                    TableView::Complete(TableRows::CountryFactories(vec![fac_ger.clone(), fac_ita.clone()]))
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

    fn load_at(source: &Source, params: Value, load: &LoadContext, cancelled: &AtomicBool) -> Value {
        let text = (crate::registrations::FACTIONS_FACTIONTOTALSAT.registered.load)(source, "test", params, load, cancelled).unwrap();
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

    fn build_late_faction_source(unobserved_factions_to: Option<i32>) -> Source {
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
                if unobserved_factions_to.is_some_and(|to| hour <= to) {
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

        let unread_src = build_late_faction_source(Some(120));
        let unread_query = dashboard_base::queries::query(&unread_src, 1).unwrap();
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
    }
}
