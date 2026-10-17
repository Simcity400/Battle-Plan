use super::*;
use dashboard_base::products::country_groups::HourSpan;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Params {
    branch_id: String,
    tag: String,
    #[serde(default)]
    view: LedgerView,
    from: Option<i32>,
    hour: Option<i32>,
    within: Option<Vec<HourSpan>>,
    navigation: Option<series::Navigation>,
    summary: Option<PartView>,
    #[serde(default)]
    totals: bool,
    bindings: Option<BTreeMap<String, Vec<HourSpan>>>,
    marks: Option<MarkView>,
    page: Option<GovernmentPage>,
    #[serde(default)]
    matches: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MarkView {
    from: f64,
    to: f64,
    width: u32,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GovernmentPage {
    from: f64,
    to: f64,
    cursor: String,
    page_size: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PartView {
    from: f64,
    to: f64,
    width: u32,
    currency: String,
    mode: String,
}

pub fn query(
    params: &Value,
    store: &store::Store,
    source: &store::SourceKey,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let params: Params = serde_json::from_value(params.clone())?;
    validate_tag(&params.tag).map_err(dashboard_core::invalid)?;
    let branch = params
        .branch_id
        .parse()
        .map_err(|_| dashboard_core::invalid("Invalid politics ledger branch"))?;
    let Some((keys, catalogue)) =
        series::read_catalogue::<Catalogue>(store, source, "politics-ledger", branch)?
    else {
        return if params.view == LedgerView::Spending {
            Ok(serde_json::to_value(PoliticsLedgerReply {
                branch_id: params.branch_id,
                tag: params.tag,
                decimals: 5,
                ..Default::default()
            })?)
        } else {
            Ok(Value::Null)
        };
    };
    let mut primary = Reader::new(
        store,
        source,
        format!("politics-ledger-{branch}"),
        cancelled,
    )?;
    if let Some(navigation) = &params.navigation
        && params.view != LedgerView::Governments
    {
        let fields = if let Some(view) = &params.summary {
            if view.mode == "income" {
                ["income", "costs", "wrapping", "income-total"]
                    .into_iter()
                    .map(|id| format!("parts:{}:{id}", view.currency))
                    .collect::<BTreeSet<_>>()
            } else {
                let mut fields =
                    BTreeSet::from([format!("parts:{}:spending-total", view.currency)]);
                for tag in params
                    .bindings
                    .as_ref()
                    .map(|bindings| bindings.keys().collect::<Vec<_>>())
                    .unwrap_or_else(|| vec![&params.tag])
                {
                    fields.extend(
                        catalogue
                            .categories
                            .get(tag)
                            .into_iter()
                            .flat_map(|categories| categories.keys())
                            .filter(|field| field.starts_with(&format!("{}:", view.currency)))
                            .map(|field| format!("parts:{field}")),
                    );
                }
                fields
            }
        } else {
            [
                "Political:income",
                "Political:coverage",
                "Command:income",
                "Command:coverage",
                "Political:events",
                "Command:events",
                "governments",
                "holds",
            ]
            .map(str::to_owned)
            .into()
        };
        let mut selected = Vec::new();
        if let Some(bindings) = &params.bindings {
            for (tag, spans) in bindings {
                for field in &fields {
                    if let Some(key) = key(&keys, tag, field) {
                        selected.push((key, Some(spans.as_slice())));
                    }
                }
            }
        } else {
            for field in &fields {
                if let Some(key) = key(&keys, &params.tag, field) {
                    selected.push((key, params.within.as_deref()));
                }
            }
        }
        return Ok(
            serde_json::json!({"hour": series::navigate(&mut [(&mut primary, selected)], navigation)?}),
        );
    }
    if let Some(view) = &params.summary {
        series::viewport(Some(view.from), Some(view.to), Some(view.width))?;
        if !matches!(view.currency.as_str(), "Political" | "Command")
            || !matches!(view.mode.as_str(), "income" | "spending")
        {
            return Err(dashboard_core::invalid("Invalid power parts selection"));
        }
        let first = catalogue.first.unwrap_or(0);
        let last = catalogue.last.unwrap_or(first);
        let mut reply = PoliticsLedgerReply {
            branch_id: params.branch_id.clone(),
            tag: params.tag.clone(),
            decimals: 5,
            ..Default::default()
        };
        let fields = if view.mode == "income" {
            let mut fields = vec![("income".to_owned(), "Daily gain".to_owned())];
            if view.currency == "Political" {
                fields.extend([
                    ("costs".into(), "Daily costs".into()),
                    ("wrapping".into(), "Adjustment".into()),
                ]);
            }
            fields
        } else {
            params
                .bindings
                .as_ref()
                .map(|bindings| bindings.keys().collect::<Vec<_>>())
                .unwrap_or_else(|| vec![&params.tag])
                .into_iter()
                .flat_map(|tag| {
                    catalogue
                        .categories
                        .get(tag)
                        .into_iter()
                        .flat_map(|categories| categories.iter())
                })
                .filter_map(|(key, label)| {
                    key.strip_prefix(&format!("{}:", view.currency))
                        .map(|id| (id.to_owned(), label.clone()))
                })
                .collect()
        };
        let mut seen = BTreeSet::new();
        let mut fields: Vec<_> = fields
            .into_iter()
            .filter(|(id, _)| seen.insert(id.clone()))
            .collect();
        fields.push((format!("{}-total", view.mode), "Total".into()));
        for (id, label) in fields {
            let field = format!("parts:{}:{id}", view.currency);
            let bindings = params.bindings.clone().unwrap_or_else(|| {
                BTreeMap::from([(
                    params.tag.clone(),
                    params.within.clone().unwrap_or_else(|| {
                        vec![HourSpan {
                            first_hour: first,
                            last_hour: last,
                        }]
                    }),
                )])
            });
            let mut spans = Vec::new();
            for (tag, within) in &bindings {
                validate_tag(tag).map_err(dashboard_core::invalid)?;
                let actual = key(&keys, tag, &field);
                let zero = (view.mode == "spending" && id != "spending-total")
                    .then(|| key(&keys, tag, &format!("parts:{}:zero", view.currency)))
                    .flatten();
                let origin = catalogue
                    .category_first
                    .get(&series::field_key(tag, &format!("{}:{id}", view.currency)))
                    .copied();
                for span in within {
                    if let Some(zero) = zero {
                        let end = origin
                            .and_then(|hour| hour.checked_sub(1))
                            .unwrap_or(span.last_hour)
                            .min(span.last_hour);
                        if span.first_hour <= end {
                            spans.push((zero, span.first_hour, end, Some(true)));
                        }
                    }
                    if let Some(actual) = actual {
                        let start = if zero.is_some() {
                            origin.unwrap_or(span.first_hour).max(span.first_hour)
                        } else {
                            span.first_hour
                        };
                        if start <= span.last_hour {
                            spans.push((actual, start, span.last_hour, Some(true)));
                        }
                    }
                }
            }
            spans.sort_by_key(|span| span.1);
            let mut previous = None;
            for span in &mut spans {
                span.3 = Some(previous.is_none_or(|last: i32| last.checked_add(1) != Some(span.1)));
                previous = Some(span.2);
            }
            let summary = Some(primary.summary_spans(
                &spans,
                (first, last),
                (view.from, view.to, view.width),
                5,
            )?);
            if let Some(summary) = summary {
                reply
                    .part_summaries
                    .push(LedgerPartSummary { id, label, summary });
            }
        }
        return Ok(serde_json::to_value(reply)?);
    }
    let first = catalogue.first.unwrap_or(0);
    let last = params.hour.or(catalogue.last).unwrap_or(first);
    let from = params.from.unwrap_or(first);
    if from > last {
        return Err(dashboard_core::invalid("Politics ledger range is reversed"));
    }
    let within = params.within.clone().unwrap_or_else(|| {
        vec![HourSpan {
            first_hour: from,
            last_hour: last,
        }]
    });
    let spans = normalize_spans(&within, from, last);
    let tail = match catalogue.tail_ranges.get(&params.tag) {
        Some(&range) => series::read_reply::<BTreeMap<i32, OpenDay>>(
            store,
            source,
            &format!("politics-ledger-tail-{branch}"),
            range,
        )?
        .unwrap_or_default(),
        None => BTreeMap::new(),
    };
    let mut native = Reader::new(
        store,
        source,
        format!("politics-ledger-recurring-{branch}"),
        cancelled,
    )?;
    if let Some(navigation) = &params.navigation {
        let mut search = series::Navigation {
            from: navigation.from,
            first_hour: navigation.first_hour,
            last_hour: navigation.last_hour,
            key: navigation.key,
        };
        let forward = matches!(
            search.key,
            series::MeasureKey::ArrowRight | series::MeasureKey::Home
        );
        let mut result = None;
        if let Some(stops) = key(&keys, &params.tag, "governments:lag")
            && let Some(key) = key(&keys, &params.tag, "governments")
        {
            loop {
                let candidate = series::navigate(
                    &mut [(&mut native, vec![(stops, params.within.as_deref())])],
                    &search,
                )?;
                let Some(candidate) = candidate.filter(|candidate| Some(*candidate) != search.from)
                else {
                    break;
                };
                let visible = native.events(&[key], candidate as i32, candidate as i32, 0, u32::MAX)?.rows.into_iter().try_fold(false, |found, event| -> io::Result<bool> {
                    Ok(found || matches!(serde_json::from_value::<Event>(event.row)?, Event::Government { row } if row.read_hour <= last))
                })?;
                if visible {
                    result = Some(candidate);
                    break;
                }
                search.from = Some(candidate);
                search.key = if forward {
                    series::MeasureKey::ArrowRight
                } else {
                    series::MeasureKey::ArrowLeft
                };
            }
        }
        let supplemental = build_reply(
            branch,
            &params.tag,
            LedgerView::Governments,
            navigation.first_hour.ceil() as i32,
            last,
            &normalize_spans(
                &[HourSpan {
                    first_hour: navigation.first_hour.ceil() as i32,
                    last_hour: navigation.last_hour.floor() as i32,
                }],
                i32::MIN,
                last,
            ),
            &keys,
            &catalogue,
            &tail,
            false,
            &mut primary,
            &mut native,
            cancelled,
            true,
        )?;
        let origin = navigation.origin()?;
        for row in supplemental.governments {
            let candidate = f64::from(row.hour);
            if (forward && candidate > origin) || (!forward && candidate < origin) {
                result = Some(result.map_or(candidate, |old| {
                    if forward {
                        old.min(candidate)
                    } else {
                        old.max(candidate)
                    }
                }));
            }
        }
        return Ok(
            serde_json::json!({"hour": result.or_else(|| navigation.from.map(|hour| hour.clamp(navigation.first_hour, navigation.last_hour)))}),
        );
    }
    if let Some(page) = &params.page {
        if params.view != LedgerView::Governments {
            return Err(dashboard_core::invalid(
                "Government event page requires governments view",
            ));
        }
        let (from, to, cursor, size) = series::event_window(
            Some(page.from),
            Some(page.to),
            Some(page.cursor.as_str()),
            Some(page.page_size),
        )?;
        let keys = key(&keys, &params.tag, "governments")
            .into_iter()
            .collect::<Vec<_>>();
        let page = native.events(&keys, from, to.min(last), cursor, size)?;
        let rows = page
            .rows
            .into_iter()
            .filter_map(|event| match serde_json::from_value::<Event>(event.row) {
                Ok(Event::Government { row }) if row.read_hour <= last => {
                    Some(Ok(dashboard_base::prepared_measure::query::MeasureEvent {
                        tag: params.tag.clone(),
                        hour: row.hour,
                        ordinal: event.ordinal.to_string(),
                        row: serde_json::to_value(row).expect("government serializes"),
                    }))
                }
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            })
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(serde_json::to_value(
            dashboard_base::prepared_measure::query::MeasureDropReply {
                coverage: None,
                first_ordinal: page.first.to_string(),
                total: page.total.to_string(),
                rows,
                warnings: Vec::new(),
                cursor: page.cursor.map(|cursor| cursor.to_string()),
            },
        )?);
    }
    if let Some(view) = &params.marks {
        if params.view != LedgerView::Governments {
            return Err(dashboard_core::invalid(
                "Government marks require governments view",
            ));
        }
        series::viewport(Some(view.from), Some(view.to), Some(view.width))?;
        let summary = match key(&keys, &params.tag, "governments") {
            Some(government) => government_marks(
                &mut native,
                government,
                key(&keys, &params.tag, "governments:lag"),
                view,
                last,
                catalogue.government_lag,
            )?,
            None => dashboard_base::prepared_chart::MarkSummary {
                from: view.from,
                to: view.to,
                width: view.width,
                clusters: Vec::new(),
            },
        };
        // Open-day and exceptionally late rows use the shared client mark adapter.
        // Their dated operands are the bounded continuation or sparse exception.
        let mut reply = build_reply(
            branch,
            &params.tag,
            LedgerView::Governments,
            view.from.ceil() as i32,
            last,
            &normalize_spans(
                &[HourSpan {
                    first_hour: view.from.ceil() as i32,
                    last_hour: view.to.floor() as i32,
                }],
                i32::MIN,
                last,
            ),
            &keys,
            &catalogue,
            &tail,
            false,
            &mut primary,
            &mut native,
            cancelled,
            true,
        )?;
        reply.government_marks = Some(summary);
        return Ok(serde_json::to_value(reply)?);
    }
    let mut reply = build_reply(
        branch,
        &params.tag,
        params.view.clone(),
        from,
        last,
        &spans,
        &keys,
        &catalogue,
        &tail,
        params.totals,
        &mut primary,
        &mut native,
        cancelled,
        false,
    )?;
    if params.view == LedgerView::Governments
        && params.matches
        && let Some(key) = key(&keys, &params.tag, "governments:supplying")
    {
        for event in primary.events(&[key], from, last, 0, u32::MAX)?.rows {
            if let Event::Government { row } = serde_json::from_value(event.row)?
                && !reply.governments.iter().any(|old| {
                    (old.country, old.instance, &old.epoch, &old.sequence)
                        == (row.country, row.instance, &row.epoch, &row.sequence)
                })
            {
                reply.governments.push(row);
            }
        }
        reply
            .governments
            .sort_by_key(|row| (row.read_hour, row.country, row.sequence.parse::<u64>().ok()));
    }
    if json_len(&reply, MAX_REPLY_BYTES).is_none() {
        return Err(dashboard_core::invalid(
            "Prepared politics ledger exceeds the complete-response budget; no rows were truncated",
        ));
    }
    Ok(serde_json::to_value(reply)?)
}

fn government_marks(
    reader: &mut Reader<'_>,
    key: u64,
    lag_key: Option<u64>,
    view: &MarkView,
    through: i32,
    lag: i32,
) -> io::Result<dashboard_base::prepared_chart::MarkSummary> {
    let mut summary = reader.event_clusters(key, view.from, view.to, view.width)?;
    let edge = through.saturating_sub(lag.max(0));
    let mut clusters = Vec::new();
    for cluster in summary.clusters {
        if cluster.last_hour <= edge {
            clusters.push(cluster);
            continue;
        }
        if cluster.first_hour == cluster.last_hour
            && let Some(lag_key) = lag_key
            && reader
                .value_at(lag_key, cluster.first_hour)?
                .is_some_and(|run| {
                    i128::from(cluster.first_hour) + run.value <= i128::from(through)
                })
        {
            clusters.push(cluster);
            continue;
        }
        let mut visible = if cluster.first_hour <= edge {
            reader
                .event_clusters(
                    key,
                    f64::from(cluster.first_hour),
                    f64::from(edge.min(cluster.last_hour)),
                    1,
                )?
                .clusters
                .into_iter()
                .next()
        } else {
            None
        };
        for event in reader
            .events(
                &[key],
                cluster.first_hour.max(edge.saturating_add(1)),
                cluster.last_hour,
                0,
                u32::MAX,
            )?
            .rows
        {
            if let Event::Government { row } = serde_json::from_value(event.row)? {
                if row.read_hour > through {
                    continue;
                }
                if let Some(old) = &mut visible {
                    old.last_hour = row.hour;
                    old.count = old
                        .count
                        .parse::<u64>()
                        .map_err(|_| dashboard_core::invalid("Government count overflow"))?
                        .checked_add(1)
                        .ok_or_else(|| dashboard_core::invalid("Government count overflow"))?
                        .to_string();
                } else {
                    visible = Some(dashboard_base::prepared_chart::MarkCluster {
                        first_hour: row.hour,
                        last_hour: row.hour,
                        first_ordinal: event.ordinal.to_string(),
                        count: "1".into(),
                    });
                }
            }
        }
        clusters.extend(visible);
    }
    summary.clusters = clusters;
    Ok(summary)
}

fn normalize_spans(within: &[HourSpan], from: i32, to: i32) -> Vec<HourSpan> {
    let mut values: Vec<_> = within
        .iter()
        .filter_map(|span| {
            let first_hour = span.first_hour.max(from);
            let last_hour = span.last_hour.min(to);
            (first_hour <= last_hour).then_some(HourSpan {
                first_hour,
                last_hour,
            })
        })
        .collect();
    values.sort_by_key(|span| span.first_hour);
    let mut result: Vec<HourSpan> = Vec::new();
    for span in values {
        if let Some(last) = result
            .last_mut()
            .filter(|last| i64::from(last.last_hour) + 1 >= i64::from(span.first_hour))
        {
            last.last_hour = last.last_hour.max(span.last_hour);
        } else {
            result.push(span);
        }
    }
    result
}
fn selected(spans: &[HourSpan], hour: i32) -> bool {
    spans
        .iter()
        .any(|span| span.first_hour <= hour && hour <= span.last_hour)
}
fn key(keys: &BTreeMap<String, u64>, tag: &str, field: &str) -> Option<u64> {
    keys.get(&series::field_key(tag, field)).copied()
}
fn state(
    reader: &mut Reader<'_>,
    keys: &BTreeMap<String, u64>,
    tag: &str,
    field: &str,
    hour: i32,
) -> io::Result<Option<i128>> {
    key(keys, tag, field)
        .map(|key| {
            reader
                .value_at(key, hour)
                .map(|run| run.map(|run| run.value))
        })
        .transpose()
        .map(Option::flatten)
}
fn first_day(hour: i32) -> i64 {
    (i64::from(hour) + 23).div_euclid(24) * 24
}
fn last_day(hour: i32) -> i64 {
    i64::from(hour).div_euclid(24) * 24
}

/// Summary totals operate on complete 24-hour native-day cells. They never
/// materialise daily payments, including when amounts change every day.
fn total(
    reader: &mut Reader<'_>,
    keys: &BTreeMap<String, u64>,
    tag: &str,
    field: &str,
    spans: &[HourSpan],
) -> io::Result<u128> {
    let Some(key) = key(keys, tag, field) else {
        return Ok(0);
    };
    let mut result = 0u128;
    for span in spans {
        let first = first_day(span.first_hour);
        let last = last_day(span.last_hour);
        if first > last {
            continue;
        }
        let end = (last + 23).min(i64::from(i32::MAX)) as i32;
        let summary = reader.summary(key, (first as i32, end), (first as f64, end as f64, 1), 0)?;
        let raw = summary
            .buckets
            .iter()
            .flatten()
            .try_fold(0u128, |value, bucket| {
                value
                    .checked_add(bucket.absolute_total)
                    .ok_or_else(|| dashboard_core::invalid("Politics summary total overflow"))
            })?;
        if raw % 24 != 0 {
            return Err(dashboard_core::invalid(
                "Incomplete politics native-day cell",
            ));
        }
        result = result
            .checked_add(raw / 24)
            .ok_or_else(|| dashboard_core::invalid("Politics summary total overflow"))?;
    }
    Ok(result)
}

fn latest_day(
    reader: &mut Reader<'_>,
    keys: &BTreeMap<String, u64>,
    tag: &str,
    field: &str,
    spans: &[HourSpan],
) -> io::Result<Option<i32>> {
    let Some(key) = key(keys, tag, field) else {
        return Ok(None);
    };
    for span in spans.iter().rev() {
        let first = first_day(span.first_hour);
        let last = last_day(span.last_hour);
        if first > last {
            continue;
        }
        let end = (last + 23).min(i64::from(i32::MAX)) as i32;
        let summary = reader.summary(key, (first as i32, end), (first as f64, end as f64, 1), 0)?;
        if let Some(bucket) = summary.buckets.iter().flatten().last() {
            return Ok(Some(last_day(bucket.last_hour) as i32));
        }
    }
    Ok(None)
}

/// Days before the maximum lag are already visible. Only the narrow right
/// edge needs exact lag runs; a read ending before publication never sees them.
fn visible(
    reader: &mut Reader<'_>,
    keys: &BTreeMap<String, u64>,
    tag: &str,
    field: &str,
    spans: &[HourSpan],
    to: i32,
    lag: i32,
) -> io::Result<Vec<HourSpan>> {
    let edge = to.saturating_sub(lag.max(0));
    let mut result = Vec::new();
    for span in spans {
        let end = span.last_hour.min(edge);
        if span.first_hour <= end {
            result.push(HourSpan {
                first_hour: span.first_hour,
                last_hour: end,
            });
        }
        let start = span.first_hour.max(edge.saturating_add(1));
        if start > span.last_hour {
            continue;
        }
        let Some(key) = key(keys, tag, field) else {
            continue;
        };
        for run in reader.runs(key, start.div_euclid(24) * 24, span.last_hour)? {
            let first = first_day(start.max(run.first_hour));
            let end = last_day(span.last_hour.min(run.last_hour)).min(
                i64::from(to)
                    - i64::try_from(run.value)
                        .map_err(|_| dashboard_core::invalid("Politics supplying lag overflow"))?,
            );
            let end = end.div_euclid(24) * 24;
            if first <= end {
                result.push(HourSpan {
                    first_hour: first as i32,
                    last_hour: end as i32,
                });
            }
        }
    }
    // The early span may end inside the same cell as an edge span.
    Ok(normalize_spans(&result, i32::MIN, i32::MAX))
}

fn month(hour: i32) -> (Option<i32>, i64) {
    use engine::cdate::{HOUR_EPOCH, MONTH_LENGTHS};
    if hour < HOUR_EPOCH {
        return (None, i64::from(hour) + 24);
    }
    let days = (i64::from(hour) - i64::from(HOUR_EPOCH)).div_euclid(24);
    let year = days.div_euclid(365);
    let mut day = days.rem_euclid(365);
    let mut index = 0;
    while day >= i64::from(MONTH_LENGTHS[index]) {
        day -= i64::from(MONTH_LENGTHS[index]);
        index += 1;
    }
    (
        Some((year * 12 + index as i64) as i32),
        i64::from(hour) - i64::from(hour).rem_euclid(24)
            + (i64::from(MONTH_LENGTHS[index]) - day) * 24,
    )
}
fn month_spans(spans: &[HourSpan]) -> Vec<(Option<i32>, Vec<HourSpan>)> {
    let mut result = BTreeMap::<_, Vec<_>>::new();
    let mut undated = Vec::new();
    for span in spans {
        let mut day = first_day(span.first_hour);
        let last = last_day(span.last_hour);
        while day <= last {
            let (month, next) = month(day as i32);
            let end = last.min(next - 24);
            let window = HourSpan {
                first_hour: day as i32,
                last_hour: end as i32,
            };
            if month.is_some() {
                result.entry(month).or_default().push(window);
            } else {
                undated.push((None, vec![window]));
            }
            day = next;
        }
    }
    undated.extend(result);
    undated
}

fn canonicalize(
    payment: &mut Payment,
    tag: &str,
    catalogue: &Catalogue,
    tips: &BTreeMap<String, String>,
) -> io::Result<()> {
    let display = display_field(payment)?;
    let origins: Vec<_> = catalogue
        .items
        .values()
        .filter(|item| item.tag == tag && item.display == display)
        .collect();
    let first = origins
        .iter()
        .filter_map(|item| item.first[0].as_ref())
        .min()
        .or_else(|| {
            origins
                .iter()
                .filter_map(|item| item.first[1].as_ref())
                .min()
        })
        .ok_or_else(|| dashboard_core::invalid("Missing politics item origin"))?;
    payment.item.id = first.2.clone();
    payment.spend.item_id = first.2.clone();
    if let Some(identity) = payment.item.tooltip_token.as_ref()
        && let Some(token) = tips.get(identity)
    {
        payment.item.tooltip_token = Some(token.clone());
    }
    if let Some(ideas) = &mut payment.ideas {
        for info in &mut ideas.ideas {
            if let Some(token) = tips.get(&info.token) {
                info.token = token.clone();
            }
        }
        for art in &mut ideas.artwork {
            if let Some(token) = tips.get(&art.token) {
                art.token = token.clone();
            }
        }
    }
    Ok(())
}
fn append_payment(
    reply: &mut PoliticsLedgerReply,
    mut payment: Payment,
    catalogue: &Catalogue,
    tips: &BTreeMap<String, String>,
    recurring: Option<(i32, i32, u32)>,
) -> io::Result<()> {
    canonicalize(&mut payment, &reply.tag, catalogue, tips)?;
    if !reply
        .spend_items
        .iter()
        .any(|item| item.id == payment.item.id)
    {
        reply.spend_items.push(payment.item);
    }
    if !reply
        .spend_categories
        .iter()
        .any(|category| category.id == payment.category.id)
    {
        reply.spend_categories.push(payment.category);
    }
    if let Some(ideas) = payment.ideas {
        let target = reply
            .idea_tooltips
            .get_or_insert_with(|| LedgerIdeaTooltips {
                ideas: Vec::new(),
                artwork: Vec::new(),
                modifiers: Vec::new(),
                texts: Vec::new(),
            });
        for info in ideas.ideas {
            if !target.ideas.iter().any(|old| old.token == info.token) {
                target.ideas.push(info);
            }
        }
        for art in ideas.artwork {
            if !target.artwork.iter().any(|old| old.token == art.token) {
                target.artwork.push(art);
            }
        }
        for modifier in ideas.modifiers {
            if !target.modifiers.iter().any(|old| old.key == modifier.key) {
                target.modifiers.push(modifier);
            }
        }
        for text in ideas.texts {
            if !target.texts.iter().any(|old| old.key == text.key) {
                target.texts.push(text);
            }
        }
    }
    if let Some((first_hour, last_hour, days)) = recurring {
        reply.recurring.push(LedgerRecurringSpend {
            spend: payment.spend,
            first_hour,
            last_hour,
            days,
            count: payment.count,
            known_raw: payment.known_raw,
            known_count: payment.known_count,
        });
    } else {
        reply.spending.push(payment.spend);
    }
    Ok(())
}
fn tips(tag: &str, catalogue: &Catalogue) -> io::Result<BTreeMap<String, String>> {
    let mut rows = Vec::new();
    for (identity, positions) in &catalogue.tooltips {
        let (found_tag, identity): (String, String) = serde_json::from_str(identity)?;
        if found_tag != tag {
            continue;
        }
        if let Some((kind, position)) = positions
            .iter()
            .enumerate()
            .find_map(|(kind, position)| position.map(|position| (kind, position)))
        {
            rows.push((kind, position, identity));
        }
    }
    rows.sort();
    Ok(rows
        .into_iter()
        .enumerate()
        .map(|(index, (_, _, identity))| (identity, format!("idea:{tag}:{index}")))
        .collect())
}

fn clip_command_payment(
    mut payment: Payment,
    spans: &[HourSpan],
    to: i32,
) -> io::Result<Option<Payment>> {
    payment.spend.changes.retain(|change| {
        change.read_hour <= to && change.hour.is_some_and(|hour| selected(spans, hour))
    });
    if payment.spend.changes.is_empty() {
        return Ok(None);
    }
    let first = payment
        .spend
        .changes
        .iter()
        .filter_map(|change| change.hour)
        .min()
        .unwrap();
    if first != payment.spend.hour {
        payment.spend.country_name = None;
    }
    payment.spend.hour = first;
    payment.spend.read_hour = payment
        .spend
        .changes
        .iter()
        .map(|change| change.read_hour)
        .max()
        .unwrap();
    payment.spend.charge_raw = sum_raw(
        payment
            .spend
            .changes
            .iter()
            .map(|change| amount(change.charge_raw.as_deref()))
            .collect::<io::Result<Vec<_>>>()?
            .into_iter(),
    )?
    .map(|value| value.to_string());
    payment.spend.realized_delta_raw = sum_raw(
        payment
            .spend
            .changes
            .iter()
            .map(|change| amount(change.realized_delta_raw.as_deref()))
            .collect::<io::Result<Vec<_>>>()?
            .into_iter(),
    )?
    .map(|value| value.to_string());
    payment.spend.occurrences = payment
        .spend
        .changes
        .iter()
        .try_fold(0u32, |total, change| {
            total
                .checked_add(change.occurrences)
                .ok_or_else(|| dashboard_core::invalid("Politics payment count overflow"))
        })?;
    payment.known_count = payment
        .spend
        .changes
        .iter()
        .filter(|change| change.charge_raw.is_some())
        .try_fold(0u32, |total, change| {
            total
                .checked_add(change.occurrences)
                .ok_or_else(|| dashboard_core::invalid("Politics payment count overflow"))
        })?;
    payment.known_raw = payment
        .spend
        .changes
        .iter()
        .filter_map(|change| change.charge_raw.as_deref())
        .try_fold(0i128, |total, raw| {
            total
                .checked_add(amount(Some(raw))?.unwrap())
                .ok_or_else(|| dashboard_core::invalid("Politics subtotal overflow"))
        })?
        .to_string();
    payment.spend.covered = payment.spend.changes.iter().all(|change| change.covered);
    Ok(Some(payment))
}
fn without_days(spans: &[HourSpan], days: &BTreeSet<i32>) -> Vec<HourSpan> {
    let mut result = Vec::new();
    for span in spans {
        let mut first = i64::from(span.first_hour);
        for &day in days.range(..=span.last_hour) {
            let day = i64::from(day);
            if day + 23 < first {
                continue;
            }
            if first < day {
                result.push(HourSpan {
                    first_hour: first as i32,
                    last_hour: (day - 1) as i32,
                });
            }
            first = first.max(day + 24);
        }
        if first <= i64::from(span.last_hour) {
            result.push(HourSpan {
                first_hour: first as i32,
                last_hour: span.last_hour,
            });
        }
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn build_reply(
    branch: u64,
    tag: &str,
    view: LedgerView,
    from: i32,
    to: i32,
    spans: &[HourSpan],
    keys: &BTreeMap<String, u64>,
    catalogue: &Catalogue,
    tail: &BTreeMap<i32, OpenDay>,
    totals_only: bool,
    primary: &mut Reader<'_>,
    native: &mut Reader<'_>,
    cancelled: &AtomicBool,
    supplemental_governments: bool,
) -> io::Result<PoliticsLedgerReply> {
    let mut reply = PoliticsLedgerReply {
        branch_id: branch.to_string(),
        tag: tag.into(),
        decimals: 5,
        ..Default::default()
    };
    let tips = tips(tag, catalogue)?;
    // The current native day lives in the tail. Summary buckets must stop at
    // the last closed cell, including when the selected window reaches the tail.
    let native_spans = catalogue.native_through.map_or_else(Vec::new, |through| {
        normalize_spans(spans, i32::MIN, through)
    });
    let mut reasons = BTreeMap::new();
    for (scope, warnings) in &catalogue.warnings {
        if scope.as_deref().is_none_or(|scope| scope == tag) {
            for (reason, count) in warnings {
                count_reason(&mut reasons, reason.clone(), *count)?;
            }
        }
    }
    if view != LedgerView::Governments {
        for (reason, count) in catalogue.presentation.get(tag).into_iter().flatten() {
            count_reason(&mut reasons, reason.clone(), *count)?;
        }
        for (reason, count) in catalogue
            .pending_presentation
            .get(tag)
            .into_iter()
            .flatten()
        {
            count_reason(&mut reasons, reason.clone(), *count)?;
        }
        for reason in catalogue.formats.get(tag).into_iter().flatten() {
            count_reason(&mut reasons, reason.clone(), 1)?;
        }
    }
    reply.warnings = reasons
        .into_iter()
        .map(|(reason, count)| format!("{reason} (count: {count})"))
        .collect();
    let fields = match view {
        LedgerView::Governments => vec!["late-governments"],
        LedgerView::Spending if totals_only => vec!["late-recurring", "late-daily"],
        LedgerView::Spending => vec![
            "Political:payments",
            "Command:payments",
            "late-recurring",
            "late-daily",
            "late-costs",
        ],
        LedgerView::Raw => vec![
            "Political:events",
            "Command:events",
            "late-recurring",
            "late-daily",
            "late-costs",
            "late-governments",
        ],
        _ => vec![
            "Political:events",
            "Command:events",
            "late-recurring",
            "late-daily",
            "late-costs",
        ],
    };
    let boundary_days: BTreeSet<_> = spans
        .iter()
        .flat_map(|span| {
            let mut days = Vec::new();
            if span.first_hour.rem_euclid(24) != 0 {
                days.push(last_day(span.first_hour) as i32);
            }
            if span.last_hour.rem_euclid(24) != 23 {
                days.push(last_day(span.last_hour) as i32);
            }
            days
        })
        .collect();
    let mut command = Vec::new();
    let event_keys: Vec<_> = fields
        .into_iter()
        .filter_map(|field| key(keys, tag, field))
        .collect();
    let events = if event_keys.is_empty() {
        Vec::new()
    } else {
        primary.events(&event_keys, from, to, 0, u32::MAX)?.rows
    };
    let mut late = Vec::new();
    for event in events {
        cancelled_request(cancelled)?;
        match serde_json::from_value::<Event>(event.row)? {
            Event::Government { row } if selected(spans, row.hour) => reply.governments.push(row),
            Event::Mutation {
                row,
                balance,
                payment,
            } => {
                if let Some(payment) = payment.as_ref().filter(|payment| {
                    payment.spend.currency == "Command" && is_recurring(&payment.spend.category)
                }) {
                    command.push(payment.clone());
                }
                if selected(spans, row.hour) {
                    if view == LedgerView::Raw {
                        reply.mutations.push(row);
                    }
                    if let Some(balance) = balance {
                        reply.balance_changes.push(balance);
                    }
                    if let Some(payment) =
                        payment.filter(|payment| !is_recurring(&payment.spend.category))
                    {
                        append_payment(&mut reply, payment, catalogue, &tips, None)?;
                    }
                }
            }
            Event::Cost { row, payment } if selected(spans, row.day_hour) => {
                if view == LedgerView::Raw {
                    reply.daily_costs.push(row);
                }
                append_payment(&mut reply, payment, catalogue, &tips, None)?;
            }
            Event::Late { payment, cost } if selected(spans, payment.spend.hour) => {
                if view == LedgerView::Raw {
                    reply.daily_costs.extend(cost);
                }
                late.push(payment);
            }
            Event::LateDaily { row } if selected(spans, row.day_hour) => reply.daily.push(row),
            Event::RawCost { row } if view == LedgerView::Raw && selected(spans, row.day_hour) => {
                reply.daily_costs.push(row)
            }
            Event::Payment { payment } if selected(spans, payment.spend.hour) => {
                append_payment(&mut reply, payment, catalogue, &tips, None)?
            }
            _ => {}
        }
    }
    if matches!(view, LedgerView::Raw | LedgerView::Governments) {
        if !supplemental_governments && let Some(key) = key(keys, tag, "governments") {
            for event in native.events(&[key], from, to, 0, u32::MAX)?.rows {
                if let Event::Government { row } = serde_json::from_value(event.row)?
                    && row.read_hour <= to
                    && selected(spans, row.hour)
                {
                    reply.governments.push(row);
                }
            }
        }
        for open in tail.values() {
            reply.governments.extend(
                open.governments
                    .iter()
                    .filter(|(_, row)| row.read_hour <= to && selected(spans, row.hour))
                    .map(|(_, row)| row.clone()),
            );
        }
        reply
            .governments
            .sort_by_key(|row| (row.read_hour, row.country, row.sequence.parse::<u64>().ok()));
        if view == LedgerView::Governments {
            return Ok(reply);
        }
    }
    if view == LedgerView::Raw
        && let Some(capture) = key(keys, tag, "capture")
        && let Some(events) = key(keys, tag, "capture:events")
    {
        let mut facts = BTreeMap::new();
        if let Some((hour, value)) = primary.latest_event(events, from)?
            && let Event::Coverage { row } = serde_json::from_value(value)?
        {
            facts.insert(hour, row);
        }
        if from < to {
            for event in primary.events(&[events], from + 1, to, 0, u32::MAX)?.rows {
                if let Event::Coverage { row } = serde_json::from_value(event.row)? {
                    facts.insert(event.hour, row);
                }
            }
        }
        for run in primary.runs(capture, from, to)? {
            // Reader joins adjacent equal values. Supplying-read facts still
            // delimit the native coverage episodes within that numeric run.
            let mut boundaries = vec![i64::from(run.first_hour)];
            boundaries.extend(
                facts
                    .range((
                        std::ops::Bound::Excluded(run.first_hour),
                        std::ops::Bound::Included(run.last_hour),
                    ))
                    .map(|(&hour, _)| i64::from(hour)),
            );
            boundaries.push(i64::from(run.last_hour) + 1);
            for pair in boundaries.windows(2) {
                let first = pair[0] as i32;
                if let Some((_, fact)) = facts.range(..=first).next_back() {
                    let mut row = fact.clone();
                    row.since_hour = Some(row.since_hour.unwrap_or(first).max(from));
                    row.through_hour = Some((pair[1] - 1) as i32);
                    reply.coverage.push(row);
                }
            }
        }
    }
    if view == LedgerView::Spending
        && let Some(key) = key(keys, tag, "Command:events")
    {
        let mut edges: Vec<_> = boundary_days
            .iter()
            .map(|&day| HourSpan {
                first_hour: day,
                last_hour: to.min(day.saturating_add(23).saturating_add(catalogue.command_lag)),
            })
            .collect();
        edges.push(HourSpan {
            first_hour: (to.saturating_sub(catalogue.command_lag)).div_euclid(24) * 24,
            last_hour: to,
        });
        for edge in normalize_spans(&edges, from.div_euclid(24) * 24, to) {
            for event in primary
                .events(&[key], edge.first_hour, edge.last_hour, 0, u32::MAX)?
                .rows
            {
                if let Event::Mutation {
                    payment: Some(payment),
                    ..
                } = serde_json::from_value(event.row)?
                    && is_recurring(&payment.spend.category)
                {
                    command.push(payment);
                }
            }
        }
    }

    let mut banks = Vec::new();
    let mut boundaries = BTreeSet::from([i64::from(from), i64::from(to) + 1]);
    let census = match key(keys, "*", "census") {
        Some(key) => primary.runs(key, from, to)?,
        None => Vec::new(),
    };
    let mut presence = BTreeMap::new();
    if let Some(key) = key(keys, "*", "census:events") {
        if let Some((hour, value)) = primary.latest_event(key, from)?
            && let Event::Presence { row } = serde_json::from_value(value)?
        {
            presence.insert(hour, row);
        }
        if from < to {
            for event in primary.events(&[key], from + 1, to, 0, u32::MAX)?.rows {
                if let Event::Presence { row } = serde_json::from_value(event.row)? {
                    presence.insert(event.hour, row);
                }
            }
        }
    }
    let mut presence_boundaries: BTreeSet<_> = census
        .iter()
        .flat_map(|run| [i64::from(run.first_hour), i64::from(run.last_hour) + 1])
        .collect();
    let mut previous_presence = None;
    for (&hour, row) in &presence {
        let value = (row.tags.contains(tag), row.refused_tags.contains(tag));
        if previous_presence != Some(value) && hour >= from {
            presence_boundaries.insert(i64::from(hour));
        }
        previous_presence = Some(value);
    }
    boundaries.extend(&presence_boundaries);
    for field in ["Political:coverage", "Command:coverage"] {
        let runs = match key(keys, tag, field) {
            Some(key) => primary.runs(key, from, to)?,
            None => Vec::new(),
        };
        boundaries.extend(runs.iter().flat_map(|run| {
            [
                i64::from(run.first_hour.max(from)),
                i64::from(run.last_hour.min(to)) + 1,
            ]
        }));
        banks.push(runs);
    }
    let mut reasons = BTreeMap::<i32, LedgerSpendCoverage>::new();
    if let Some(key) = key(keys, tag, "coverage-reason") {
        if let Some((hour, value)) = primary.latest_event(key, from)? {
            reasons.insert(hour, serde_json::from_value(value)?);
        }
        if from < to {
            for event in primary
                .events(&[key], from.saturating_add(1), to, 0, u32::MAX)?
                .rows
            {
                boundaries.insert(i64::from(event.hour));
                reasons.insert(event.hour, serde_json::from_value(event.row)?);
            }
        }
    }
    let boundaries: Vec<_> = boundaries.into_iter().collect();
    let mut positions = [0usize; 2];
    for pair in boundaries.windows(2) {
        let first = pair[0] as i32;
        let last = (pair[1] - 1) as i32;
        let mut known = [false; 2];
        let supplied = census
            .iter()
            .any(|run| run.first_hour <= first && first <= run.last_hour);
        let present = supplied
            && presence
                .range(..=first)
                .next_back()
                .is_some_and(|(_, row)| row.tags.contains(tag));
        let absent = supplied
            && presence
                .range(..=first)
                .next_back()
                .is_some_and(|(_, row)| !row.tags.contains(tag) && !row.refused_tags.contains(tag));
        for (index, runs) in banks.iter().enumerate() {
            while positions[index] < runs.len() && runs[positions[index]].last_hour < first {
                positions[index] += 1;
            }
            known[index] = runs
                .get(positions[index])
                .is_some_and(|run| run.first_hour <= first && run.last_hour >= first);
            known[index] |= absent;
        }
        let fact = reasons
            .range(..=first)
            .next_back()
            .map(|(_, fact)| fact)
            .filter(|fact| present && fact.political == known[0] && fact.command == known[1]);
        let value = LedgerSpendCoverage {
            // An unchanged reason can cross a gap; its supplying read belongs
            // only to the interval recorded in that fact.
            read_hour: fact
                .filter(|fact| {
                    fact.since_hour
                        .zip(fact.through_hour)
                        .is_some_and(|(since, through)| since <= first && first <= through)
                })
                .map_or(first, |fact| fact.read_hour),
            since_hour: Some(first),
            through_hour: Some(last),
            political: known[0],
            command: known[1],
            reason: fact.and_then(|fact| fact.reason.clone()).or_else(|| {
                (!(known[0] && known[1])).then(|| "politics coverage unavailable".into())
            }),
        };
        if presence_boundaries.contains(&i64::from(first)) {
            reply.spending_coverage.push(value);
        } else {
            merge_spending_coverage(&mut reply.spending_coverage, value);
        }
    }
    if matches!(view, LedgerView::Raw | LedgerView::Balance)
        && let Some(key) = key(keys, tag, "holds")
    {
        let mut snapshots = Vec::new();
        if let Some((hour, event)) = primary.latest_event(key, from)?
            && let Event::Hold { row } = serde_json::from_value(event)?
        {
            snapshots.push((hour, row));
        }
        if from < to {
            for event in primary.events(&[key], from + 1, to, 0, u32::MAX)?.rows {
                if let Event::Hold { row } = serde_json::from_value(event.row)? {
                    snapshots.push((event.hour, row));
                }
            }
        }
        for (hour, mut row) in snapshots {
            let at = hour.max(from);
            let present = census
                .iter()
                .any(|run| run.first_hour <= at && at <= run.last_hour)
                && presence
                    .range(..=at)
                    .next_back()
                    .is_some_and(|(_, row)| row.tags.contains(tag));
            // Boundary facts close a snapshot even when its value stayed the
            // same (for example, a zero hold at the first absence read).
            if present {
                row.through_hour = to;
            }
            if let Some(previous) = reply.command_holds.last_mut() {
                if previous.read_hour == row.read_hour {
                    previous.through_hour = row.through_hour;
                    continue;
                }
                previous.through_hour = previous.through_hour.min(row.read_hour - 1);
            }
            reply.command_holds.push(row);
        }
        // A hold carries over accepted hours, never the missing interval
        // between its supplying episode and the next observed snapshot.
        if let Some(first) = reply.command_holds.first().map(|row| row.read_hour)
            && let Some(key) = self::key(keys, "*", "accepted")
        {
            let accepted = primary.runs(key, first, to)?;
            for row in &mut reply.command_holds {
                let index = accepted.partition_point(|run| run.last_hour < row.read_hour);
                if let Some(run) = accepted
                    .get(index)
                    .filter(|run| run.first_hour <= row.read_hour)
                {
                    row.through_hour = row.through_hour.min(run.last_hour);
                }
            }
        }
    }
    if totals_only {
        for item in catalogue
            .items
            .values()
            .filter(|item| item.tag == tag && !is_recurring(&item.category.id))
        {
            let field = item_field(&item.facts)?;
            let Some(key) = key(keys, tag, &format!("{field}:totals")) else {
                continue;
            };
            let mut selected_totals = Totals::default();
            for span in spans {
                let Some((_, value)) = primary.latest_event(key, span.last_hour)? else {
                    continue;
                };
                let Event::Totals { totals: end, .. } = serde_json::from_value(value)? else {
                    return Err(dashboard_core::invalid("Invalid politics endpoint"));
                };
                let start = if span.first_hour == i32::MIN {
                    Totals::default()
                } else {
                    match primary.latest_event(key, span.first_hour - 1)? {
                        Some((_, value)) => match serde_json::from_value(value)? {
                            Event::Totals { totals, .. } => totals,
                            _ => return Err(dashboard_core::invalid("Invalid politics endpoint")),
                        },
                        None => Totals::default(),
                    }
                };
                let mut range = subtract_totals(&end, &start)?;
                let edge = span.last_hour.min(span.first_hour.saturating_add(item.lag));
                for event in primary
                    .events(&[key], span.first_hour, edge, 0, u32::MAX)?
                    .rows
                {
                    let Event::Totals { payment, .. } = serde_json::from_value(event.row)? else {
                        return Err(dashboard_core::invalid("Invalid politics endpoint"));
                    };
                    if payment.spend.hour < span.first_hour {
                        let mut excluded = Totals::default();
                        excluded.add(&payment)?;
                        range = subtract_totals(&range, &excluded)?;
                    }
                }
                selected_totals.known = selected_totals
                    .known
                    .checked_add(range.known)
                    .ok_or_else(|| dashboard_core::invalid("Politics subtotal overflow"))?;
                selected_totals.count = selected_totals
                    .count
                    .checked_add(range.count)
                    .ok_or_else(|| dashboard_core::invalid("Politics aggregate count overflow"))?;
                selected_totals.rows = selected_totals
                    .rows
                    .checked_add(range.rows)
                    .ok_or_else(|| dashboard_core::invalid("Politics aggregate count overflow"))?;
                selected_totals.unknown = selected_totals
                    .unknown
                    .checked_add(range.unknown)
                    .ok_or_else(|| dashboard_core::invalid("Politics aggregate count overflow"))?;
                selected_totals.uncovered = selected_totals
                    .uncovered
                    .checked_add(range.uncovered)
                    .ok_or_else(|| dashboard_core::invalid("Politics aggregate count overflow"))?;
                if range.rows != 0
                    && let Some(latest) = end.latest
                    && selected_totals
                        .latest
                        .as_ref()
                        .is_none_or(|old| old.spend.hour <= latest.spend.hour)
                {
                    selected_totals.latest = Some(latest);
                }
            }
            if selected_totals.rows == 0 {
                continue;
            }
            let mut payment = selected_totals
                .latest
                .ok_or_else(|| dashboard_core::invalid("Missing politics latest payment"))?;
            payment.spend.id = format!("total:{tag}:{field}");
            payment.spend.charge_raw =
                (selected_totals.unknown == 0).then(|| selected_totals.known.to_string());
            payment.spend.realized_delta_raw = None;
            payment.spend.occurrences = u32::try_from(selected_totals.count)
                .map_err(|_| dashboard_core::invalid("Politics payment count overflow"))?;
            payment.spend.covered = selected_totals.uncovered == 0;
            payment.known_raw = selected_totals.known.to_string();
            payment.known_count = u32::try_from(selected_totals.count - selected_totals.unknown)
                .map_err(|_| dashboard_core::invalid("Politics payment count overflow"))?;
            payment.count = u32::try_from(selected_totals.rows)
                .map_err(|_| dashboard_core::invalid("Politics row count overflow"))?;
            payment.spend.changes = vec![LedgerSpendChange {
                hour: Some(payment.spend.hour),
                read_hour: payment.spend.read_hour,
                charge_raw: payment.spend.charge_raw.clone(),
                realized_delta_raw: None,
                occurrences: payment.spend.occurrences,
                covered: payment.spend.covered,
            }];
            let last = payment.spend.hour;
            append_payment(&mut reply, payment, catalogue, &tips, Some((from, last, 0)))?;
        }
    }

    for item in catalogue
        .items
        .values()
        .filter(|item| item.tag == tag && is_recurring(&item.category.id))
    {
        cancelled_request(cancelled)?;
        let field = item_field(&item.facts)?;
        let visible = visible(
            native,
            keys,
            tag,
            &format!("{field}:lag"),
            &native_spans,
            to,
            item.lag,
        )?;
        let visible = if item.currency == "Command" {
            without_days(&visible, &boundary_days)
        } else {
            visible
        };
        let windows = if totals_only {
            vec![(None, visible)]
        } else {
            month_spans(&visible).into_iter().collect()
        };
        for (month, spans) in windows {
            let count = total(native, keys, tag, &format!("{field}:count"), &spans)?;
            if count == 0 {
                continue;
            }
            let days = total(native, keys, tag, &format!("{field}:presence"), &spans)?;
            let charge = total(native, keys, tag, &format!("{field}:charge"), &spans)?;
            let unknown = total(native, keys, tag, &format!("{field}:unknown"), &spans)?;
            let uncovered = total(native, keys, tag, &format!("{field}:uncovered"), &spans)?;
            let native_hour = latest_day(native, keys, tag, &format!("{field}:presence"), &spans)?
                .ok_or_else(|| dashboard_core::invalid("Missing politics payment date"))?;
            let read_hour = native_hour.saturating_add(
                i32::try_from(
                    state(native, keys, tag, &format!("{field}:lag"), native_hour)?.unwrap_or(0),
                )
                .map_err(|_| dashboard_core::invalid("Politics supplying lag overflow"))?,
            );
            let facts_key = key(keys, tag, &format!("{field}:facts"));
            let mut payment = match facts_key
                .map(|key| primary.latest_event(key, read_hour))
                .transpose()?
                .flatten()
            {
                Some((_, value)) => match serde_json::from_value(value)? {
                    Event::Item { payment } => payment,
                    _ => return Err(dashboard_core::invalid("Invalid politics item facts")),
                },
                None => item.facts.clone(),
            };
            payment.spend.id = format!("recurring:{tag}:{field}:{month:?}:{}", spans[0].first_hour);
            payment.spend.hour = native_hour.saturating_add(
                i32::try_from(
                    state(native, keys, tag, &format!("{field}:hour"), native_hour)?.unwrap_or(0),
                )
                .map_err(|_| dashboard_core::invalid("Politics native payment hour overflow"))?,
            );
            payment.spend.read_hour = read_hour;
            payment.spend.sequence =
                state(native, keys, tag, &format!("{field}:sequence"), native_hour)?
                    .unwrap_or(0)
                    .to_string();
            payment.spend.ordinal = u32::try_from(
                state(native, keys, tag, &format!("{field}:ordinal"), native_hour)?.unwrap_or(0),
            )
            .map_err(|_| dashboard_core::invalid("Politics cost ordinal overflow"))?;
            payment.spend.charge_raw = (unknown == 0).then(|| charge.to_string());
            payment.known_raw = charge.to_string();
            payment.known_count = u32::try_from(count - unknown)
                .map_err(|_| dashboard_core::invalid("Politics payment count overflow"))?;
            payment.spend.realized_delta_raw = None;
            if matches!(view, LedgerView::Raw | LedgerView::Balance) || month.is_none() {
                let unknown = total(
                    native,
                    keys,
                    tag,
                    &format!("{field}:realized-unknown"),
                    &spans,
                )?;
                if unknown == 0 {
                    let positive = i128::try_from(total(
                        native,
                        keys,
                        tag,
                        &format!("{field}:realized-positive"),
                        &spans,
                    )?)
                    .map_err(|_| dashboard_core::invalid("Politics realized total overflow"))?;
                    let negative = i128::try_from(total(
                        native,
                        keys,
                        tag,
                        &format!("{field}:realized-negative"),
                        &spans,
                    )?)
                    .map_err(|_| dashboard_core::invalid("Politics realized total overflow"))?;
                    payment.spend.realized_delta_raw = Some(
                        positive
                            .checked_sub(negative)
                            .ok_or_else(|| {
                                dashboard_core::invalid("Politics realized total overflow")
                            })?
                            .to_string(),
                    );
                }
            }
            payment.spend.occurrences = u32::try_from(count)
                .map_err(|_| dashboard_core::invalid("Politics payment count overflow"))?;
            payment.spend.covered = uncovered == 0;
            payment.spend.changes = vec![LedgerSpendChange {
                hour: Some(payment.spend.hour),
                read_hour: payment.spend.read_hour,
                charge_raw: payment.spend.charge_raw.clone(),
                realized_delta_raw: payment.spend.realized_delta_raw.clone(),
                occurrences: payment.spend.occurrences,
                covered: payment.spend.covered,
            }];
            payment.count =
                u32::try_from(total(native, keys, tag, &format!("{field}:rows"), &spans)?)
                    .map_err(|_| dashboard_core::invalid("Politics row count overflow"))?;
            append_payment(
                &mut reply,
                payment,
                catalogue,
                &tips,
                Some((
                    spans[0].first_hour,
                    native_hour,
                    u32::try_from(days)
                        .map_err(|_| dashboard_core::invalid("Politics day count overflow"))?,
                )),
            )?;
        }
    }
    for open in tail.values() {
        for payment in open.payments.values() {
            if payment.spend.read_hour <= to
                && selected(spans, payment.spend.hour)
                && !(payment.spend.currency == "Command"
                    && boundary_days.contains(&(payment.spend.hour.div_euclid(24) * 24)))
            {
                late.push(payment.clone());
            }
        }
        for row in open.daily.values() {
            if selected(spans, row.day_hour)
                && row
                    .pp_read_hour
                    .into_iter()
                    .chain(row.cp_read_hour)
                    .chain([row.read_hour])
                    .any(|read| read <= to)
            {
                reply.daily.push(row.clone());
            }
        }
        if view == LedgerView::Raw {
            reply.daily_costs.extend(
                open.costs
                    .values()
                    .filter(|row| row.read_hour <= to && selected(spans, row.day_hour))
                    .cloned(),
            );
        }
    }
    let mut commands = BTreeMap::<String, Payment>::new();
    let mut command_ids = BTreeSet::new();
    let mut replaced_commands = BTreeSet::new();
    for payment in command {
        if !command_ids.insert(payment.spend.id.clone()) {
            continue;
        }
        let Some(payment) = clip_command_payment(payment, spans, to)? else {
            continue;
        };
        let day = payment.spend.hour.div_euclid(24) * 24;
        let field = item_field(&payment)?;
        let committed = !boundary_days.contains(&day)
            && state(native, keys, tag, &format!("{field}:presence"), day)?.is_some()
            && state(native, keys, tag, &format!("{field}:lag"), day)?
                .is_some_and(|lag| i128::from(day) + lag <= i128::from(to));
        let in_tail = !boundary_days.contains(&day)
            && tail.get(&day).is_some_and(|open| {
                open.payments.values().any(|old| {
                    old.identity == payment.identity
                        && old.spend.category == payment.spend.category
                        && old.spend.read_hour <= to
                        && selected(spans, old.spend.hour)
                })
            });
        if committed || in_tail {
            continue;
        }
        replaced_commands.insert(payment.spend.id.clone());
        let id = serde_json::to_string(&(&payment.identity, &payment.spend.category, day))?;
        match commands.entry(id) {
            std::collections::btree_map::Entry::Occupied(mut old) => {
                merge_payment(old.get_mut(), payment)?
            }
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(payment);
            }
        }
    }
    late.retain(|payment| {
        payment.spend.currency != "Command" || !replaced_commands.contains(&payment.spend.id)
    });
    late.extend(commands.into_values());
    for mut payment in late {
        let day = payment.spend.hour.div_euclid(24) * 24;
        if payment.spend.category == "upkeep" && !boundary_days.contains(&day) {
            let mut existing = None;
            for item in catalogue.items.values().filter(|item| {
                item.tag == tag
                    && item.category.id == "upkeep"
                    && item.facts.identity == payment.identity
            }) {
                let field = item_field(&item.facts)?;
                if state(native, keys, tag, &format!("{field}:presence"), day)?.is_some()
                    && let Some(index) = reply.recurring.iter().position(|row| {
                        row.spend
                            .id
                            .starts_with(&format!("recurring:{tag}:{field}:"))
                    })
                {
                    existing = Some((index, item));
                    break;
                }
            }
            if let Some((index, item)) = existing {
                // Production keeps one upkeep row for this capture actor/day,
                // including a later supplying read behind the closed frontier.
                let row = &mut reply.recurring[index];
                let mut combined = item.facts.clone();
                combined.spend = row.spend.clone();
                combined.count = row.count;
                combined.known_raw = row.known_raw.clone();
                combined.known_count = row.known_count;
                let date = combined.spend.hour;
                let name = combined.spend.country_name.clone();
                merge_payment(&mut combined, payment)?;
                combined.spend.hour = date;
                combined.spend.country_name = name;
                combined.spend.changes = vec![LedgerSpendChange {
                    hour: Some(date),
                    read_hour: combined.spend.read_hour,
                    charge_raw: combined.spend.charge_raw.clone(),
                    realized_delta_raw: combined.spend.realized_delta_raw.clone(),
                    occurrences: combined.spend.occurrences,
                    covered: combined.spend.covered,
                }];
                row.spend = combined.spend;
                row.known_raw = combined.known_raw;
                row.known_count = combined.known_count;
                continue;
            }
        }
        payment.spend.changes = vec![LedgerSpendChange {
            hour: Some(payment.spend.hour),
            read_hour: payment.spend.read_hour,
            charge_raw: payment.spend.charge_raw.clone(),
            realized_delta_raw: payment.spend.realized_delta_raw.clone(),
            occurrences: payment.spend.occurrences,
            covered: payment.spend.covered,
        }];
        append_payment(&mut reply, payment, catalogue, &tips, Some((day, day, 1)))?;
    }

    let mut latest_daily = BTreeMap::new();
    for row in std::mem::take(&mut reply.daily) {
        latest_daily.insert(
            (row.country, row.instance, row.epoch.clone(), row.day_hour),
            row,
        );
    }
    reply.daily = latest_daily.into_values().collect();

    let categories: BTreeSet<_> = catalogue
        .items
        .values()
        .filter(|item| item.tag == tag && is_recurring(&item.category.id))
        .map(|item| (item.currency.clone(), item.category.id.clone()))
        .collect();
    for (currency, category) in categories.into_iter().filter(|_| !totals_only) {
        let field = format!("category:{currency}:{category}");
        let lag = catalogue
            .items
            .values()
            .filter(|item| {
                item.tag == tag && item.currency == currency && item.category.id == category
            })
            .map(|item| item.lag)
            .max()
            .unwrap_or(0);
        let visible = visible(
            native,
            keys,
            tag,
            &format!("{field}:lag"),
            &native_spans,
            to,
            lag,
        )?;
        let visible = if currency == "Command" {
            without_days(&visible, &boundary_days)
        } else {
            visible
        };
        let mut months = BTreeMap::<Option<i32>, u32>::new();
        for (month, window) in month_spans(&visible) {
            let count = total(native, keys, tag, &format!("{field}:days"), &window)?;
            if count != 0 {
                months.insert(
                    month,
                    u32::try_from(count)
                        .map_err(|_| dashboard_core::invalid("Politics day count overflow"))?,
                );
            }
        }
        let days: BTreeSet<_> = reply
            .recurring
            .iter()
            .filter(|row| {
                row.spend.currency == currency
                    && row.spend.category == category
                    && row.first_hour == row.last_hour
            })
            .map(|row| row.first_hour)
            .collect();
        for day in days {
            let published = !(currency == "Command" && boundary_days.contains(&day))
                && state(native, keys, tag, &format!("{field}:days"), day)?.is_some()
                && state(native, keys, tag, &format!("{field}:lag"), day)?
                    .is_some_and(|lag| i128::from(day) + lag <= i128::from(to));
            if !published {
                let count = months.entry(month(day).0).or_default();
                *count = count
                    .checked_add(1)
                    .ok_or_else(|| dashboard_core::invalid("Politics day count overflow"))?;
            }
        }
        reply
            .recurring_days
            .extend(months.into_iter().map(|(month, days)| LedgerMonthDays {
                currency: currency.clone(),
                category: category.clone(),
                month,
                days,
            }));
    }

    for currency in ["Political", "Command"] {
        let mut gained = 0i128;
        let mut lost = 0u128;
        let mut incomplete = 0u128;
        let mut any = false;
        for actor in catalogue.actors.get(tag).into_iter().flatten() {
            let prefix = daily_prefix(actor)?;
            let lag = catalogue
                .daily_lags
                .get(&series::field_key(tag, &prefix))
                .copied()
                .unwrap_or_default()[usize::from(currency == "Command")];
            let visible = visible(
                native,
                keys,
                tag,
                &format!("{prefix}:{currency}:availability-lag"),
                &native_spans,
                to,
                lag,
            )?;
            let positive = total(
                native,
                keys,
                tag,
                &format!("{prefix}:{currency}:gained-positive"),
                &visible,
            )?;
            let negative = total(
                native,
                keys,
                tag,
                &format!("{prefix}:{currency}:gained-negative"),
                &visible,
            )?;
            gained = gained
                .checked_add(
                    i128::try_from(positive)
                        .map_err(|_| dashboard_core::invalid("Politics gain overflow"))?,
                )
                .and_then(|value| value.checked_sub(i128::try_from(negative).ok()?))
                .ok_or_else(|| dashboard_core::invalid("Politics gain overflow"))?;
            lost = lost
                .checked_add(total(
                    native,
                    keys,
                    tag,
                    &format!("{prefix}:{currency}:lost"),
                    &visible,
                )?)
                .ok_or_else(|| dashboard_core::invalid("Politics cap total overflow"))?;
            incomplete = incomplete
                .checked_add(total(
                    native,
                    keys,
                    tag,
                    &format!("{prefix}:{currency}:incomplete"),
                    &visible,
                )?)
                .ok_or_else(|| dashboard_core::invalid("Politics incomplete-day count overflow"))?;
            any |= total(
                native,
                keys,
                tag,
                &format!("{prefix}:{currency}:days"),
                &visible,
            )? != 0;
            for row in reply.daily.iter().filter(|row| {
                (row.country, row.instance, &row.epoch) == (actor.0, actor.1, &actor.2)
            }) {
                // A sparse late tuple replaces this actor's earlier native cell.
                if selected(&visible, row.day_hour) {
                    incomplete = incomplete
                        .checked_sub(
                            state(
                                native,
                                keys,
                                tag,
                                &format!("{prefix}:{currency}:incomplete"),
                                row.day_hour,
                            )?
                            .unwrap_or(0) as u128,
                        )
                        .ok_or_else(|| {
                            dashboard_core::invalid("Politics incomplete-day count underflow")
                        })?;
                    let positive = state(
                        native,
                        keys,
                        tag,
                        &format!("{prefix}:{currency}:gained-positive"),
                        row.day_hour,
                    )?
                    .unwrap_or(0);
                    let negative = state(
                        native,
                        keys,
                        tag,
                        &format!("{prefix}:{currency}:gained-negative"),
                        row.day_hour,
                    )?
                    .unwrap_or(0);
                    gained = gained
                        .checked_sub(positive)
                        .and_then(|value| value.checked_sub(negative))
                        .ok_or_else(|| dashboard_core::invalid("Politics gain overflow"))?;
                    lost = lost
                        .checked_sub(
                            state(
                                native,
                                keys,
                                tag,
                                &format!("{prefix}:{currency}:lost"),
                                row.day_hour,
                            )?
                            .unwrap_or(0) as u128,
                        )
                        .ok_or_else(|| dashboard_core::invalid("Politics cap total underflow"))?;
                }
                let (gain, net, applied, read) = if currency == "Political" {
                    (
                        amount(row.pp_gross_raw.as_deref())?,
                        amount(row.pp_income_raw.as_deref())?,
                        amount(row.pp_realized_raw.as_deref())?,
                        row.pp_read_hour,
                    )
                } else {
                    (
                        amount(row.cp_income_raw.as_deref())?,
                        amount(row.cp_income_raw.as_deref())?,
                        amount(row.cp_realized_raw.as_deref())?,
                        row.cp_read_hour,
                    )
                };
                if read.unwrap_or(row.read_hour) > to {
                    continue;
                }
                any |= read.is_some();
                if let Some(gain) = gain.filter(|_| read.is_some()) {
                    gained = gained
                        .checked_add(if currency == "Command" {
                            gain.max(0)
                        } else {
                            gain
                        })
                        .ok_or_else(|| dashboard_core::invalid("Politics gain overflow"))?;
                } else {
                    incomplete = incomplete.checked_add(1).ok_or_else(|| {
                        dashboard_core::invalid("Politics incomplete-day count overflow")
                    })?;
                }
                if read.is_some()
                    && let Some((net, applied)) = net
                        .zip(applied)
                        .filter(|(net, applied)| *net > 0 && applied < net)
                {
                    lost = lost
                        .checked_add((net - applied.max(0)) as u128)
                        .ok_or_else(|| dashboard_core::invalid("Politics cap total overflow"))?;
                }
            }
        }
        if any {
            reply.gains.push(LedgerGain {
                currency: currency.into(),
                gained_raw: gained.to_string(),
                lost_raw: lost.to_string(),
                complete: incomplete == 0,
            });
        }
    }
    if view == LedgerView::Raw || view == LedgerView::Balance {
        for span in spans {
            let mut day = first_day(span.first_hour);
            let last = last_day(span.last_hour);
            while day <= last {
                for actor in catalogue.actors.get(tag).into_iter().flatten() {
                    if let Some(row) =
                        read_daily(native, keys, tag, &daily_prefix(actor)?, day as i32)?
                        && !reply.daily.iter().any(|old| {
                            old.country == row.country
                                && old.instance == row.instance
                                && old.epoch == row.epoch
                                && old.day_hour == row.day_hour
                        })
                    {
                        reply.daily.push(row);
                    }
                }
                if view == LedgerView::Raw {
                    for (id, template) in &catalogue.costs {
                        let (found_tag, field): (String, String) = serde_json::from_str(id)?;
                        if found_tag != tag {
                            continue;
                        }
                        let Some(amount) =
                            state(native, keys, tag, &format!("{field}:amount"), day as i32)?
                        else {
                            continue;
                        };
                        let mut row = template.clone();
                        row.day_hour = day as i32;
                        row.amount_raw = amount.to_string();
                        row.pp_sequence =
                            state(native, keys, tag, &format!("{field}:sequence"), day as i32)?
                                .unwrap_or(0)
                                .to_string();
                        row.read_hour = row.day_hour.saturating_add(
                            i32::try_from(
                                state(native, keys, tag, &format!("{field}:lag"), day as i32)?
                                    .unwrap_or(0),
                            )
                            .map_err(|_| dashboard_core::invalid("Politics cost lag overflow"))?,
                        );
                        row.covered =
                            state(native, keys, tag, &format!("{field}:covered"), day as i32)?
                                .unwrap_or(0)
                                != 0;
                        if row.read_hour <= to {
                            reply.daily_costs.push(row);
                        }
                    }
                }
                day += 24;
            }
        }
        for row in &mut reply.daily {
            mask_daily(row, to);
        }
        reply.daily.retain(|row| row.read_hour <= to);
        reply
            .daily
            .sort_by_key(|row| (row.day_hour, row.country, row.instance));
    } else {
        reply.daily.clear();
    }
    reply
        .daily_costs
        .sort_by_key(|row| (row.day_hour, row.country, row.instance, row.ordinal));
    reply.daily_costs.dedup_by(|a, b| {
        a.country == b.country
            && a.instance == b.instance
            && a.epoch == b.epoch
            && a.day_hour == b.day_hour
            && a.ordinal == b.ordinal
    });
    reply.spend_categories.sort_by(|a, b| a.id.cmp(&b.id));
    reply
        .governments
        .sort_by_key(|row| (row.read_hour, row.country, row.sequence.parse::<u64>().ok()));
    reply.spending.sort_by_key(|row| {
        (
            row.hour,
            row.sequence.parse::<u64>().ok(),
            row.ordinal,
            row.id.clone(),
        )
    });
    Ok(reply)
}

fn read_daily(
    reader: &mut Reader<'_>,
    keys: &BTreeMap<String, u64>,
    tag: &str,
    prefix: &str,
    day: i32,
) -> io::Result<Option<LedgerDaily>> {
    let Some(country) = state(reader, keys, tag, &format!("{prefix}:country"), day)? else {
        return Ok(None);
    };
    let mut values = BTreeMap::new();
    for field in DAILY_FIELDS {
        values.insert(
            *field,
            state(reader, keys, tag, &format!("{prefix}:{field}"), day)?,
        );
    }
    let raw = |field: &str| values[field].map(|value| value.to_string());
    let read = |field: &str| -> io::Result<Option<i32>> {
        values[field]
            .map(|lag| {
                i32::try_from(i128::from(day) + lag)
                    .map_err(|_| dashboard_core::invalid("Politics daily lag overflow"))
            })
            .transpose()
    };
    Ok(Some(LedgerDaily {
        country: u32::try_from(country)
            .map_err(|_| dashboard_core::invalid("Politics country overflow"))?,
        instance: u32::try_from(values["instance"].unwrap_or(0))
            .map_err(|_| dashboard_core::invalid("Politics instance overflow"))?,
        epoch: raw("epoch").unwrap_or_else(|| "0".into()),
        day_hour: day,
        read_hour: i32::try_from(values["read"].unwrap_or(i128::from(day)))
            .map_err(|_| dashboard_core::invalid("Politics read hour overflow"))?,
        pp_read_hour: read("Political:lag")?,
        cp_read_hour: read("Command:lag")?,
        pp_gross_raw: raw("Political:gross"),
        pp_cost_raw: raw("Political:cost"),
        pp_income_raw: raw("Political:income"),
        pp_realized_raw: raw("Political:realized"),
        pp_sequence: raw("Political:sequence"),
        cp_income_raw: raw("Command:income"),
        cp_realized_raw: raw("Command:realized"),
        cp_sequence: raw("Command:sequence"),
    }))
}
fn mask_daily(row: &mut LedgerDaily, to: i32) {
    if row.pp_read_hour.is_some_and(|read| read > to) {
        row.pp_read_hour = None;
        row.pp_gross_raw = None;
        row.pp_cost_raw = None;
        row.pp_income_raw = None;
        row.pp_realized_raw = None;
        row.pp_sequence = None;
    }
    if row.cp_read_hour.is_some_and(|read| read > to) {
        row.cp_read_hour = None;
        row.cp_income_raw = None;
        row.cp_realized_raw = None;
        row.cp_sequence = None;
    }
    if row.read_hour > to
        && row
            .pp_read_hour
            .into_iter()
            .chain(row.cp_read_hour)
            .any(|read| read <= to)
    {
        row.read_hour = row
            .pp_read_hour
            .into_iter()
            .chain(row.cp_read_hour)
            .filter(|read| *read <= to)
            .max()
            .unwrap_or(row.read_hour);
    }
}

fn subtract_totals(end: &Totals, start: &Totals) -> io::Result<Totals> {
    Ok(Totals {
        known: end
            .known
            .checked_sub(start.known)
            .ok_or_else(|| dashboard_core::invalid("Politics subtotal overflow"))?,
        count: end
            .count
            .checked_sub(start.count)
            .ok_or_else(|| dashboard_core::invalid("Politics payment count underflow"))?,
        rows: end
            .rows
            .checked_sub(start.rows)
            .ok_or_else(|| dashboard_core::invalid("Politics row count underflow"))?,
        unknown: end
            .unknown
            .checked_sub(start.unknown)
            .ok_or_else(|| dashboard_core::invalid("Politics payment count underflow"))?,
        uncovered: end
            .uncovered
            .checked_sub(start.uncovered)
            .ok_or_else(|| dashboard_core::invalid("Politics payment count underflow"))?,
        latest: end.latest.clone(),
    })
}
