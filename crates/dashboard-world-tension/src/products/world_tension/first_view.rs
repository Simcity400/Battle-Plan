use super::*;
use dashboard_base::{
    prepared::{
        self,
        query::{keyframes, progressive},
        series::{self, Run},
    },
    products::{ContentOptions, country_history::RecordedHistories},
};
use serde_json::{Value, json};
use std::sync::Arc;

const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::WorldTension,
    TableId::ThreatCountryAggregates,
];

#[cfg(test)]
#[derive(Default)]
struct AmountWork(usize, usize);

#[cfg(test)]
fn amount_work(source: &Source, indexed: usize, probes: usize) {
    let work = source.state::<AmountWork>();
    let mut work = work.lock().unwrap();
    work.0 += indexed;
    work.1 += probes;
}

struct Observation {
    tension: i128,
    sum: i128,
    amounts: HashMap<(String, u32), i128>,
}

#[allow(clippy::too_many_arguments)]
fn summary_tail(
    source: &Source,
    branch: u64,
    after: i32,
    value: &Value,
    data: &mut Value,
    cancelled: &AtomicBool,
) -> io::Result<Vec<dashboard_base::products::country_groups::HourSpan>> {
    let view = series::viewport(
        value["from"].as_f64(),
        value["to"].as_f64(),
        value["width"]
            .as_u64()
            .and_then(|width| u32::try_from(width).ok()),
    )?;
    let bounds = source
        .metadata()
        .branches
        .iter()
        .find(|item| item.id.parse::<u64>().ok() == Some(branch))
        .map(|item| (item.first_hour, item.last_hour))
        .ok_or_else(|| dashboard_core::invalid("Unknown tension branch"))?;
    let mut selections = BTreeMap::new();
    let mut tails = BTreeMap::from([
        ("world".to_owned(), Vec::new()),
        ("sum".to_owned(), Vec::new()),
    ]);
    if let Some(within) = value["within"].as_object() {
        for (id, selected) in within {
            let selected: Vec<(String, i32, i32, bool)> = serde_json::from_value(selected.clone())?;
            if selected.iter().any(|(_, first, last, _)| first > last)
                || selected.windows(2).any(|pair| pair[0].2 >= pair[1].1)
            {
                return Err(dashboard_core::invalid("Invalid dated series spans"));
            }
            let mut spans = Vec::new();
            for (key, first, last, _) in selected {
                let native = if key.starts_with('[') {
                    Some(serde_json::from_str::<(String, u32)>(&key)?)
                } else {
                    key.parse::<usize>()
                        .map_err(|_| dashboard_core::invalid("Invalid tension series key"))?;
                    data["catalogue"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .find(|item| item["key"].as_str() == Some(key.as_str()))
                        .and_then(|item| {
                            Some((
                                item["tag"].as_str()?.to_owned(),
                                u32::try_from(item["slot"].as_u64()?).ok()?,
                            ))
                        })
                };
                if let Some(native) = native {
                    spans.push((native, first, last));
                }
            }
            selections.insert(id.clone(), spans);
            tails.insert(id.clone(), Vec::new());
        }
    }
    let share = value["share"].as_bool().unwrap_or(false);
    let approximate = keyframes::after(
        source,
        "world-tension:keyframe",
        branch,
        after.max((view.0.floor() as i32).saturating_sub(1)),
        Some(view.1.floor() as i32),
        TABLES,
        cancelled,
        &|frame| {
            let reply = at_frame(frame, source.replay_ref().format)?;
            let Some(read) = reply.reads.first() else {
                return Ok(None);
            };
            let sample = Observation {
                tension: read.tension_raw.parse().expect("native tension integer"),
                sum: read.sum_raw.parse().expect("native contribution sum"),
                amounts: reply
                    .countries
                    .into_iter()
                    .filter_map(|country| {
                        country.runs.first().map(|run| {
                            #[cfg(test)]
                            amount_work(source, 1, 0);
                            (
                                (country.tag.clone(), country.slot),
                                run.value_raw.parse().expect("native contribution integer"),
                            )
                        })
                    })
                    .collect(),
            };
            Ok(Some(sample))
        },
        &|sample: &Observation| {
            std::mem::size_of::<Observation>()
                + sample.amounts.capacity() * 2 * std::mem::size_of::<((String, u32), i128)>()
                + sample
                    .amounts
                    .iter()
                    .map(|((tag, _), _)| tag.capacity())
                    .sum::<usize>()
        },
        |spans, sample| {
            let world = sample.tension * i128::from(cworld_threat::SCALAR_DIVISOR);
            for span in spans {
                tails.get_mut("world").unwrap().push(Run::new(
                    span.first_hour,
                    span.last_hour,
                    world,
                    true,
                ));
                tails.get_mut("sum").unwrap().push(Run::new(
                    span.first_hour,
                    span.last_hour,
                    if share { world } else { sample.sum },
                    true,
                ));
            }
            for (id, selected) in &selections {
                for (native, first, last) in selected {
                    // Absence in one frame cannot prove a historical zero.
                    #[cfg(test)]
                    amount_work(source, 0, 1);
                    let Some(&raw) = sample.amounts.get(native) else {
                        continue;
                    };
                    let amount = if share {
                        contribution_share(raw, sample.tension, sample.sum)?
                    } else {
                        raw
                    };
                    for span in spans {
                        let start = (*first).max(span.first_hour);
                        let end = (*last).min(span.last_hour);
                        if start <= end {
                            tails
                                .get_mut(id)
                                .unwrap()
                                .push(Run::new(start, end, amount, true));
                        }
                    }
                }
            }
            Ok(())
        },
    )?;
    for (id, tail) in tails {
        let mut summary = match data["summaries"].get(&id) {
            Some(summary) => serde_json::from_value(summary.clone())?,
            None => dashboard_base::prepared_chart::ChartSummary {
                from: view.0,
                to: view.1,
                width: view.2,
                buckets: Vec::new(),
                runs: Some(Vec::new()),
                approximate: Vec::new(),
            },
        };
        series::merge_tail(&mut summary, bounds, &tail, None, 5, None)?;
        let sampled: Vec<_> = tail
            .iter()
            .map(|run| dashboard_base::products::country_groups::HourSpan {
                first_hour: run.first_hour,
                last_hour: run.last_hour,
            })
            .collect();
        progressive::clip_approximate(&mut summary, &sampled, None);
        data["summaries"][&id] = serde_json::to_value(summary)?;
    }
    if !approximate.is_empty() {
        data["available"] = json!(true);
    }
    Ok(approximate)
}

/// The frame's independent total and named current contributions use the same
/// extraction as preparation. A missing remembered tag is a pending contributor,
/// never proof that its amount is unnamed or that an earlier entry became zero.
fn at_frame(
    frame: &DecodedFrame,
    format: replay::format::FormatVersion,
) -> io::Result<TensionReply> {
    let mut scan = Scan::default();
    scan.refresh_holders(frame, true, true);
    scan.refresh_inputs(frame, true, false);
    scan.observe_inputs(format, frame, true, true, false)?;
    let known = scan.known.clone();
    let mut result = scan.finish();
    result.events.clear();
    result.unnamed_reads = 0;
    result.partial_sources = 0;
    let mut history = dashboard_base::products::country_history::HistoryScan::new(false, &[]);
    history.observe(frame);
    let mut reply = result.reply(
        &BranchParams {
            branch_id: frame.branch.to_string(),
        },
        &history.finish(),
    )?;
    for read in &mut reply.reads {
        read.preparing = vec!["historicalContributors".into(), "startsSegment".into()];
    }
    if !reply.reads.is_empty() {
        reply.preparing_contributors = aggregate_operands(frame)
            .into_iter()
            .flatten()
            .filter(|(slot, total)| *total != 0 && !known.contains_key(slot))
            .map(|(slot, total)| PreparingContributor {
                hour: frame.hour,
                slot,
                value_raw: total.to_string(),
            })
            .collect();
        if !reply.preparing_contributors.is_empty() {
            reply.reads[0].preparing.push("unnamedRaw".into());
            reply.reads[0].unnamed_raw = None;
        }
    }
    Ok(reply)
}

pub fn query(
    source: &Source,
    content: Option<&ContentOptions>,
    product: &str,
    value: &Value,
    store: &store::Store,
    key: &store::SourceKey,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let params: BranchParams = serde_json::from_value(json!({"branchId":value["branchId"]}))?;
    params.validate().map_err(dashboard_core::invalid)?;
    let branch = params.branch();
    let coverage =
        prepared::coverage::product_coverage(source, content, branch, "world-tension", cancelled)?;
    let complete_history = dashboard_base::products::country_history::prepared_branch_histories(
        source, branch, content, cancelled,
    )?;
    let identities_preparing = complete_history.is_none();
    let history = match complete_history {
        Some(history) => history,
        None => {
            let accepted = prepared::coverage::product_coverage(
                source,
                content,
                branch,
                "country-identities",
                cancelled,
            )?;
            Arc::new(progressive::histories(
                store,
                key,
                branch,
                accepted.as_ref(),
                &BTreeMap::new(),
                &BTreeMap::new(),
                source.replay_ref(),
                &[],
                cancelled,
            )?)
        }
    };
    let mut data = super::prepared::query(source, product, value, store, key, &history, cancelled)?;
    if product == "series-next-change" {
        return Ok(data);
    }
    if data.is_null() {
        data = serde_json::to_value(
            ScannedTension::default().reply(&params, &RecordedHistories::default())?,
        )?;
        data["available"] = json!(false);
        data["catalogue"] = json!([]);
        data["summaries"] = json!({});
        data["exactHours"] = json!([]);
    }
    let after = coverage.as_ref().map_or_else(
        || {
            source
                .block_windows(branch)
                .first()
                .map_or(i32::MIN, |span| span.first_hour.saturating_sub(1))
        },
        |coverage| coverage.last_hour,
    );
    let mut approximate = Vec::new();
    if product == "series-summary" && !coverage.as_ref().is_some_and(|coverage| coverage.complete) {
        approximate = summary_tail(source, branch, after, value, &mut data, cancelled)?;
    }
    if product == "series-state" && !coverage.as_ref().is_some_and(|coverage| coverage.complete) {
        let hours = value["reads"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|read| {
                read["hour"]
                    .as_i64()
                    .and_then(|hour| i32::try_from(hour).ok())
                    .ok_or_else(|| dashboard_core::invalid("Invalid tension read hour"))
            })
            .collect::<io::Result<BTreeSet<_>>>()?;
        for hour in hours {
            if hour <= after {
                continue;
            }
            let Some(frame) = progressive::frame_at(source, branch, hour, TABLES, cancelled)?
            else {
                continue;
            };
            let current = at_frame(&frame, source.replay_ref().format)?;
            if current.reads.is_empty() {
                continue;
            }
            data["available"] = json!(true);
            data["reads"].as_array_mut().expect("tension reads").extend(
                serde_json::to_value(&current.reads)?
                    .as_array()
                    .unwrap()
                    .iter()
                    .cloned(),
            );
            for country in current.countries {
                let existing = data["countries"]
                    .as_array_mut()
                    .expect("tension countries")
                    .iter_mut()
                    .find(|item| item["tag"] == country.tag && item["slot"] == country.slot);
                if let Some(existing) = existing {
                    existing["runs"].as_array_mut().unwrap().extend(
                        serde_json::to_value(&country.runs)?
                            .as_array()
                            .unwrap()
                            .iter()
                            .cloned(),
                    );
                    if !existing["identities"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|identity| {
                            identity["firstHour"]
                                .as_i64()
                                .is_some_and(|first| first <= i64::from(hour))
                                && identity["lastHour"]
                                    .as_i64()
                                    .is_some_and(|last| last >= i64::from(hour))
                        })
                    {
                        existing["identities"].as_array_mut().unwrap().extend(
                            serde_json::to_value(&country.identities)?
                                .as_array()
                                .unwrap()
                                .iter()
                                .cloned(),
                        );
                    }
                } else {
                    let sample_key = serde_json::to_string(&(&country.tag, country.slot))?;
                    data["catalogue"].as_array_mut().unwrap().push(json!({"key":sample_key,"tag":country.tag,"slot":country.slot,"firstHour":hour}));
                    data["countries"]
                        .as_array_mut()
                        .unwrap()
                        .push(serde_json::to_value(country)?);
                }
            }
            let pending = data
                .as_object_mut()
                .unwrap()
                .entry("preparingContributors")
                .or_insert_with(|| json!([]));
            pending.as_array_mut().unwrap().extend(
                serde_json::to_value(current.preparing_contributors)?
                    .as_array()
                    .unwrap()
                    .iter()
                    .cloned(),
            );
        }
    }
    if !coverage.as_ref().is_some_and(|coverage| coverage.complete) {
        let mut pending = prepared::prefix::pending(
            source,
            branch,
            after,
            value["to"].as_f64().map(|hour| hour.floor() as i32),
        );
        for span in approximate {
            pending = pending
                .into_iter()
                .flat_map(|part| {
                    if part.last_hour < span.first_hour || part.first_hour > span.last_hour {
                        return vec![part];
                    }
                    let mut left = Vec::new();
                    if part.first_hour < span.first_hour {
                        left.push(dashboard_base::products::country_groups::HourSpan {
                            first_hour: part.first_hour,
                            last_hour: span.first_hour - 1,
                        });
                    }
                    if part.last_hour > span.last_hour {
                        left.push(dashboard_base::products::country_groups::HourSpan {
                            first_hour: span.last_hour + 1,
                            last_hour: part.last_hour,
                        });
                    }
                    left
                })
                .collect();
        }
        data["pending"] = serde_json::to_value(pending)?;
        data["preparing"] = json!(["events", "historicalContributors"]);
    }
    if identities_preparing {
        data.as_object_mut()
            .unwrap()
            .entry("preparing")
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .unwrap()
            .push(json!("identities"));
    }
    data["coverage"] = serde_json::to_value(progressive::identity_coverage(
        source,
        content,
        branch,
        coverage.as_ref(),
        !identities_preparing,
        cancelled,
    )?)?;
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::super::test_fixtures::tension_frame;
    use super::*;

    #[test]
    fn contributor_queries_index_each_sample_once_and_probe_by_native_key() {
        contributor_lookup_work(128);
    }

    #[cfg(feature = "slow-tests")]
    #[test]
    fn contributor_queries_scale_to_five_thousand_native_holders() {
        contributor_lookup_work(5000);
    }

    fn contributor_lookup_work(count: u32) {
        use dashboard_base::test_fixtures::{file_source, replay};
        use replay::frame::TableView;
        use replay::rows::{CountryLinkRow, CountryRow, TableRows, ThreatCountryAggregateRow};
        let mut frame = tension_frame(100, true, 100, &[], &[]);
        *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
            (0..count)
                .map(|i| CountryRow {
                    id: i + 1,
                    canonical_slot: i,
                    tag: format!("holder {i}").into_bytes(),
                    ..Default::default()
                })
                .collect(),
        ));
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(
                (0..count)
                    .map(|i| CountryLinkRow {
                        id: i + 1,
                        canonical_slot: i,
                    })
                    .collect(),
            ));
        *frame.tables[TableId::ThreatCountryAggregates.index()] =
            TableView::Complete(TableRows::ThreatCountryAggregates(
                (0..count)
                    .map(|i| ThreatCountryAggregateRow {
                        country_slot: i,
                        total_raw: i64::from(i + 1),
                    })
                    .collect(),
            ));
        let (source, root) = file_source(replay(vec![frame]));
        let within: serde_json::Map<_, _> = (0..count)
            .map(|i| {
                (
                    format!("line {i}"),
                    json!([[
                        serde_json::to_string(&(format!("holder {i}"), i)).unwrap(),
                        100,
                        100,
                        false
                    ]]),
                )
            })
            .collect();
        let value = json!({"from":100,"to":100,"width":1,"within":within});

        for asks in 1..=2 {
            let mut data = json!({"catalogue":[],"summaries":{},"available":false});
            summary_tail(&source, 1, 99, &value, &mut data, &AtomicBool::new(false)).unwrap();
            for i in 0..count {
                assert!(
                    !data["summaries"][format!("line {i}")]["runs"]
                        .as_array()
                        .unwrap()
                        .is_empty()
                );
            }
            assert_eq!(
                {
                    let work = source.state::<AmountWork>();
                    let work = work.lock().unwrap();
                    (work.0, work.1)
                },
                (count as usize, asks * count as usize),
                "a retained sample is indexed once, with one lookup per contributor per ask"
            );
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn summary_reasks_reuse_keyframes_and_keep_absent_contributors_unknown() {
        use dashboard_base::test_fixtures::{file_source, replay};
        let first = tension_frame(100, true, 100, &[(7, 6000), (9, 4000)], &[]);
        let current = at_frame(&first, replay::format::FormatVersion::CURRENT).unwrap();
        let named = current
            .countries
            .iter()
            .find(|country| !country.runs.is_empty())
            .unwrap();
        let native = serde_json::to_string(&(&named.tag, named.slot)).unwrap();
        let expected =
            named.runs[0].value_raw.parse::<f64>().unwrap() / f64::from(current.contribution_scale);
        let branch = first.branch;
        let last = tension_frame(101, false, 200, &[(7, 10000)], &[]);
        let (source, root) = file_source(replay(vec![first, last]));
        let value = json!({"from":100, "to":101, "width":2, "within": {
            "named":[[native,100,101,false]], "absent":[["[\"unseen\",123]",100,101,false]] }});
        for after in [99, 100] {
            let mut data = json!({"catalogue":[], "summaries":{}, "available":false});
            let approximate = summary_tail(
                &source,
                branch,
                after,
                &value,
                &mut data,
                &AtomicBool::new(false),
            )
            .unwrap();
            assert_eq!(approximate[0].first_hour, after + 1);
            assert_eq!(data["available"], true);
            assert_eq!(
                data["summaries"]["named"]["runs"][0]["value"]
                    .as_f64()
                    .unwrap(),
                expected
            );
            assert!(
                data["summaries"]["absent"]["runs"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        }
        assert_eq!(
            dashboard_base::prepared::query::observations::usage(&source, "world-tension:keyframe")
                .2,
            1
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn frame_contributions_match_preparation_and_disappeared_names_stay_pending() {
        let first = tension_frame(100, true, 100, &[(7, 6000), (9, 4000)], &[]);
        let last = tension_frame(101, false, 100, &[(7, 6000), (9, 4000)], &[]);
        let format = replay::format::FormatVersion::CURRENT;
        let mut prepared = Scan::default();
        prepared.observe(format, &first).unwrap();
        prepared.observe(format, &last).unwrap();
        let expected = prepared.finish();
        let cold = at_frame(&last, format).unwrap();
        assert_eq!(
            cold.reads[0].tension_raw,
            expected.reads.last().unwrap().tension_raw
        );
        assert_eq!(
            cold.reads[0].sum_raw,
            expected.reads.last().unwrap().sum_raw
        );
        assert_eq!(cold.reads[0].unnamed_raw, None);
        assert!(
            cold.reads[0]
                .preparing
                .iter()
                .any(|field| field == "unnamedRaw")
        );
        assert_eq!(cold.preparing_contributors.len(), 1);
        assert_eq!(
            (
                cold.preparing_contributors[0].slot,
                cold.preparing_contributors[0].value_raw.as_str()
            ),
            (9, "4000")
        );
        for country in &cold.countries {
            let full = expected
                .countries
                .iter()
                .find(|full| full.tag == country.tag && full.slot == country.slot)
                .unwrap();
            assert_eq!(
                country.runs[0].value_raw,
                full.runs.last().unwrap().value_raw
            );
        }
        assert!(
            cold.events.is_empty(),
            "cross-frame source history is not inferred"
        );
    }
}
