use super::*;
use dashboard_base::{
    prepared::{
        coverage::Coverage,
        query::{keyframes, progressive},
    },
    products::country_history::{HistoryScan, RecordedHistories},
};

type Identities = BTreeMap<(String, String), Vec<(i32, i32, u32, u32)>>;

#[derive(Default)]
pub(super) struct Tail {
    pub history: History,
    pub identities: Identities,
    pub approximate: Vec<HourSpan>,
    pub answered: Vec<HourSpan>,
}

struct Sample {
    history: History,
    identities: Vec<(String, u32, u32)>,
}

fn add_identities(out: &mut Identities, history: &RecordedHistories, spans: &[HourSpan]) {
    for (tag, country) in &history.countries {
        for identity in country.identities.iter() {
            for span in spans {
                let first = span.first_hour.max(identity.first_hour);
                let last = span.last_hour.min(identity.last_hour);
                if first <= last {
                    out.entry((tag.clone(), String::new())).or_default().push((
                        first,
                        last,
                        identity.slot,
                        identity.country_id,
                    ));
                }
            }
        }
    }
}

pub(super) fn tail(
    source: &Source,
    context: &LoadContext,
    product: &str,
    params: &Params,
    coverage: Option<&Coverage>,
    cancelled: &AtomicBool,
) -> io::Result<Tail> {
    let mut tail = Tail::default();
    if coverage.is_some_and(|coverage| coverage.complete) {
        return Ok(tail);
    }
    let branch: u64 = params
        .branch_id
        .parse()
        .map_err(|_| dashboard_core::invalid("Invalid occupation branch"))?;
    if product == "series-state" {
        let [read] = params.reads.as_slice() else {
            return Err(dashboard_core::invalid(
                "Occupation state needs one reached hour",
            ));
        };
        if coverage.is_some_and(|coverage| read.hour <= coverage.last_hour) {
            return Ok(tail);
        }
        let Some(window) = source
            .block_windows(branch)
            .iter()
            .position(|span| span.first_hour <= read.hour && read.hour <= span.last_hour)
        else {
            return Ok(tail);
        };
        let Some(observation) = observed_window(source, context, branch, window, cancelled)? else {
            return Ok(tail);
        };
        let span = HourSpan {
            first_hour: read.hour,
            last_hour: read.hour,
        };
        for (tag, runs) in &observation.trend.series {
            if let Some(run) = runs
                .iter()
                .find(|run| run.first_hour <= read.hour && read.hour <= run.last_hour)
            {
                tail.history.series.insert(
                    tag.clone(),
                    vec![OccupationTrendRun {
                        first_hour: read.hour,
                        last_hour: read.hour,
                        starts_segment: run.starts_segment && run.first_hour == read.hour,
                        ..run.clone()
                    }],
                );
            }
        }
        add_identities(&mut tail.identities, &observation.identities, &[span]);
        tail.answered.push(span);
        return Ok(tail);
    }
    if product != "series-summary" {
        return Ok(tail);
    }
    let view = series::viewport(params.from, params.to, params.width)?;
    let after = coverage
        .map_or(i32::MIN, |coverage| coverage.last_hour)
        .max((view.0.floor() as i32).saturating_sub(1));
    let (_, binding) = dashboard_base::prepared::files(source, context.content.as_ref())?;
    let tables = observed_tables(false);
    tail.approximate = keyframes::blocks_after(
        source,
        "occupation:first-read",
        branch,
        after,
        Some(view.1.floor() as i32),
        &tables,
        &(),
        Some(&binding),
        cancelled,
        &|sample: &Sample| {
            history_bytes(&sample.history)
                + sample.identities.capacity() * std::mem::size_of::<(String, u32, u32)>()
                + sample
                    .identities
                    .iter()
                    .map(|(tag, _, _)| tag.capacity())
                    .sum::<usize>()
        },
        &|block| {
            // Metrics locate the first supplying read without decoding occupation
            // deltas for the rest of the block. Only that hour needs its rows.
            let replay = source.replay_ref();
            let frames = replay.read_block_selected(block, &[])?;
            let Some(read) = frames.iter().find(|frame| {
                slot_read(
                    replay.format,
                    &frame.metrics,
                    TableId::StateResistance,
                    frame.hour,
                ) && !replay.gaps.iter().any(|gap| {
                    gap.branch == branch
                        && gap.first_hour <= frame.hour
                        && frame.hour <= gap.last_hour
                })
            }) else {
                return Ok(None);
            };
            let mut prepared = Prepared::default();
            prepared.configure(source, context);
            let tables = observed_tables(prepared.reads_trigger_world());
            let Some(frame) = progressive::frame_at(source, branch, read.hour, &tables, cancelled)?
            else {
                return Ok(None);
            };
            prepared.observe_frame(&frame, replay.format, None)?;
            if !prepared.scan.holds_read() {
                return Ok(None);
            }
            let mut identities = HistoryScan::new(false, &[]);
            identities.observe(&frame);
            let history = prepared.scan.trend.history(Some(read.hour));
            let identities = identities.finish();
            let identities = history
                .series
                .keys()
                .filter_map(|tag| {
                    identities
                        .countries
                        .get(tag)
                        .and_then(|country| country.identities.first())
                        .map(|identity| (tag.clone(), identity.slot, identity.country_id))
                })
                .collect();
            Ok(Some(Sample {
                history,
                identities,
            }))
        },
        |spans, sample| {
            for (tag, runs) in &sample.history.series {
                let Some(run) = runs.last() else {
                    continue;
                };
                for span in spans {
                    tail.history
                        .series
                        .entry(tag.clone())
                        .or_default()
                        .push(OccupationTrendRun {
                            first_hour: span.first_hour,
                            last_hour: span.last_hour,
                            starts_segment: true,
                            ..run.clone()
                        });
                }
            }
            for (tag, slot, id) in &sample.identities {
                for span in spans {
                    tail.identities
                        .entry((tag.clone(), String::new()))
                        .or_default()
                        .push((span.first_hour, span.last_hour, *slot, *id));
                }
            }
            Ok(true)
        },
    )?;
    tail.answered = tail.approximate.clone();
    Ok(tail)
}

pub(super) fn numeric(history: &History, tag: &str, column: usize) -> Vec<Run> {
    history
        .series
        .get(tag)
        .into_iter()
        .flatten()
        .map(|run| {
            Run::new(
                run.first_hour,
                run.last_hour,
                if column == 0 {
                    &run.resistance_raw
                } else {
                    &run.compliance_raw
                }
                .parse()
                .expect("validated occupation fixed5"),
                run.starts_segment,
            )
        })
        .collect()
}

fn strings(values: &Vec<String>) -> usize {
    values.capacity() * std::mem::size_of::<String>()
        + values.iter().map(String::capacity).sum::<usize>()
}
fn history_bytes(history: &History) -> usize {
    std::mem::size_of::<History>() + strings(&history.warnings) + strings(&history.collaboration_warnings)
        + history.series.iter().map(|(tag, runs)| {
            tag.capacity() + 2 * std::mem::size_of::<(String, Vec<OccupationTrendRun>)>()
                + runs.capacity() * std::mem::size_of::<OccupationTrendRun>()
                + runs.iter().map(|run| run.resistance_raw.capacity() + run.compliance_raw.capacity()
                    + run.collaboration.capacity() * std::mem::size_of::<occupation_trend::OccupationCollaboration>()
                    + run.collaboration.iter().map(|item| item.tag.as_ref().map_or(0, String::capacity)
                        + item.raw.as_ref().map_or(0, String::capacity)).sum::<usize>()).sum::<usize>()
        }).sum::<usize>()
        + history.segments.iter().map(|(tag, spans)| tag.capacity()
            + 2 * std::mem::size_of::<(String, Vec<HourSpan>)>()
            + spans.capacity() * std::mem::size_of::<HourSpan>()).sum::<usize>()
}
fn identity_bytes(history: &RecordedHistories) -> usize {
    use dashboard_base::products::country_history::{
        BalanceHistory, CountryIdentityRun, NumericBalanceDrop, NumericRun, RecordedCountry,
    };
    std::mem::size_of::<RecordedHistories>()
        + history.identity_reads.capacity() * std::mem::size_of::<HourSpan>()
        + history
            .countries
            .iter()
            .map(|(tag, country)| {
                tag.capacity()
                    + 2 * std::mem::size_of::<(String, RecordedCountry)>()
                    + 5 * 2 * std::mem::size_of::<usize>()
                    + std::mem::size_of::<Vec<CountryIdentityRun>>()
                    + country.identities.capacity() * std::mem::size_of::<CountryIdentityRun>()
                    + std::mem::size_of::<Vec<HourSpan>>()
                    + country.segments.capacity() * std::mem::size_of::<HourSpan>()
                    + std::mem::size_of::<Vec<NumericRun>>()
                    + country.territory.capacity() * std::mem::size_of::<NumericRun>()
                    + [&country.political_power, &country.command_power]
                        .into_iter()
                        .map(|balance| {
                            std::mem::size_of::<BalanceHistory>()
                                + balance.runs.capacity() * std::mem::size_of::<NumericRun>()
                                + balance.drops.capacity()
                                    * std::mem::size_of::<NumericBalanceDrop>()
                        })
                        .sum::<usize>()
            })
            .sum::<usize>()
}

pub(super) fn window_bytes(window: &WindowObservation) -> usize {
    fn pairs<T>(values: &Vec<T>) -> usize {
        values.capacity() * std::mem::size_of::<T>()
    }
    let reply = &window.states;
    std::mem::size_of::<WindowObservation>()
        + reply.branch_id.capacity()
        + strings(&reply.texts)
        + strings(&reply.warnings)
        + pairs(&reply.observed)
        + pairs(&reply.states)
        + reply
            .states
            .iter()
            .map(|state| {
                let fixed = [
                    &state.resistance,
                    &state.compliance,
                    &state.resistance_speed,
                    &state.compliance_speed,
                ]
                .into_iter()
                .map(|values| {
                    pairs(values)
                        + values
                            .iter()
                            .map(|run| {
                                [&run.raw, &run.deltas, &run.delta_scale]
                                    .into_iter()
                                    .flatten()
                                    .map(String::capacity)
                                    .sum::<usize>()
                            })
                            .sum::<usize>()
                })
                .sum::<usize>();
                fixed
                    + [
                        &state.name,
                        &state.name_missing,
                        &state.owner,
                        &state.occupier,
                        &state.occupied,
                        &state.garrison_missing,
                        &state.law,
                        &state.law_missing,
                    ]
                    .into_iter()
                    .map(pairs)
                    .sum::<usize>()
                    + [&state.garrison_strength_raw, &state.garrison_required_raw]
                        .into_iter()
                        .map(|values| {
                            pairs(values)
                                + values
                                    .iter()
                                    .filter_map(|(_, value)| value.as_ref())
                                    .map(String::capacity)
                                    .sum::<usize>()
                        })
                        .sum::<usize>()
                    + pairs(&state.required_manpower)
                    + pairs(&state.assigned_manpower)
                    + pairs(&state.garrison_alert)
                    + pairs(&state.garrison_red)
            })
            .sum::<usize>()
        + reply.laws.as_ref().map_or(0, |laws| {
            pairs(laws)
                + laws
                    .iter()
                    .map(|law| {
                        law.token.capacity()
                            + [&law.name, &law.icon, &law.missing]
                                .into_iter()
                                .flatten()
                                .map(String::capacity)
                                .sum::<usize>()
                    })
                    .sum::<usize>()
        })
        + history_bytes(&window.trend)
        + identity_bytes(&window.identities)
}
