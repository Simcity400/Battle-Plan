//! Occupation on the segment driver: what its operators and emit share for a run of the
//! recording ([`Binding`]), and the replies read from the family's series publication: replay-
//! block windows, the manifest, and the occupiers' series.
use super::*;
use crate::products::occupation_trend::{
    self, History, OccupationMeasure, OccupationTop, OccupationTrendParams, OccupationTrendRun,
};
use crate::products::occupied_states::columnar::emit::{MEANS, NAME, POSITIONS, RANKS, SCHEMA};
use dashboard_base::prepared::{engine, published, series};
use dashboard_text::metadata::check_cancel;
use serde_json::{Value, json};
use std::sync::{LazyLock, Mutex, PoisonError};

mod read;
#[cfg(test)]
mod series_tests;

/// What occupation's operators and emits share over a run: the recorded content stack's
/// catalogue and the law, name and garrison content bound from it, and the texts they name.
pub struct Binding {
    catalogue: Option<Arc<Catalogue>>,
    scan: Mutex<scan::Scan<'static>>,
    texts: Arc<columnar::Texts>,
    world: bool,
}

impl Binding {
    /// The content the recorded mod stack gives, or a warning naming why it gives none.
    pub fn load(source: &Source, context: &LoadContext) -> Self {
        let catalogue = context
            .content()
            .and_then(|options| catalogue::recorded_catalogue(source, options));
        let mut scan = scan::Scan::new(catalogue.as_ref().ok().map(|catalogue| &**catalogue))
            .bind(None);
        if let Err(error) = &catalogue {
            scan.result.warnings.push(format!(
                "State names and occupation law artwork unavailable: {error}"
            ));
        }
        Self::bound(catalogue.ok(), scan)
    }

    fn bound(catalogue: Option<Arc<Catalogue>>, scan: scan::Scan<'static>) -> Self {
        Self {
            catalogue,
            world: scan.reads_trigger_world(),
            scan: Mutex::new(scan),
            texts: Arc::default(),
        }
    }

    fn scan(&self) -> std::sync::MutexGuard<'_, scan::Scan<'static>> {
        self.scan.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The tables its operators read: their own and, when a law's eligibility can read the
    /// trigger world, the shared trigger kit.
    pub fn tables(&self) -> &'static [TableId] {
        tables(self.world)
    }

    /// What a projection reads of its walk's world.
    pub fn needs(&self) -> dashboard_world::Needs {
        columnar::needs()
    }

    /// A projection of one columnar segment.
    pub fn projector(
        &self,
        format: replay_data::format::FormatVersion,
        mode: replay_data::scope::CaptureMode,
    ) -> io::Result<columnar::Projector> {
        columnar::Projector::new(
            format,
            mode,
            self.catalogue.clone(),
            self.scan().cold(),
            Arc::clone(&self.texts),
        )
    }

    /// The ordered emit of one branch's projected hours.
    pub fn emit(&self) -> columnar::Emit {
        columnar::Emit::new(
            self.catalogue.clone(),
            &self.scan(),
            catalogue::OwnRefusals::default(),
            Arc::clone(&self.texts),
        )
    }

    /// The emit `saved` holds, going on with the publication `directory` reads.
    pub fn resume(
        &self,
        directory: &dashboard_series::family::Directory,
        saved: &[u8],
    ) -> io::Result<columnar::Emit> {
        columnar::Emit::load(
            self.catalogue.clone(),
            &self.scan(),
            Arc::clone(&self.texts),
            directory,
            saved,
        )
    }
}

/// The tables the observation reads: those of [`scan`] and, when a law's eligibility can read
/// the trigger world, the shared trigger kit.
pub(super) fn observed_tables(world: bool) -> Vec<TableId> {
    let mut tables = vec![
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::States,
        TableId::StateResistance,
        TableId::StateGarrisons,
        TableId::StateOccupationLaws,
        TableId::OccupationSettings,
        TableId::StateAssignedManpower,
        TableId::CountryCollaboration,
        TableId::LexicalTokens,
    ];
    if world {
        tables.extend_from_slice(dashboard_text::triggers::RECORDED_TABLES);
    }
    tables.sort_unstable();
    tables.dedup();
    tables
}

/// [`observed_tables`], each list made once.
pub fn tables(world: bool) -> &'static [TableId] {
    static OWN: LazyLock<Vec<TableId>> = LazyLock::new(|| observed_tables(false));
    static WORLD: LazyLock<Vec<TableId>> = LazyLock::new(|| observed_tables(true));
    if world { WORLD.as_slice() } else { OWN.as_slice() }
}

/// `occupied-states` (one window) or `occupied-states-manifest` from the family's publication:
/// a window is null before the family publishes the branch; the manifest lists the branch's
/// blocks, with no law or warning before it does.
pub fn window_query(
    source: &Source,
    context: &LoadContext,
    product: &str,
    value: Value,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    check_cancel(cancelled)?;
    let content = context.content.as_ref();
    if product == "occupied-states" {
        let params: OccupiedStatesParams = serde_json::from_value(value)?;
        let branch = params
            .branch_id
            .parse()
            .map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let interval = source
            .block_windows(branch)
            .get(params.window)
            .cloned()
            .ok_or_else(|| {
                dashboard_core::invalid("Occupied states window not in this recording")
            })?;
        let Some(opened) = published::open(source, content, NAME, SCHEMA, branch)? else {
            return Ok(Value::Null);
        };
        let reply = opened.read(cancelled, |snapshot, directory| {
            read::window(snapshot, directory, &params.branch_id, params.window, interval)
        })?;
        return Ok(serde_json::to_value(reply)?);
    }
    let params: BranchParams = serde_json::from_value(value)?;
    params.validate().map_err(dashboard_core::invalid)?;
    let branch = params.branch();
    let listing = match published::open(source, content, NAME, SCHEMA, branch)? {
        Some(opened) => opened.read(cancelled, |_, directory| read::listing(directory))?,
        None => None,
    }
    .unwrap_or_default();
    let mut reply = OccupiedStatesManifestReply {
        branch_id: branch.to_string(),
        decimals: 5,
        windows: source.block_windows(branch).to_vec(),
        laws: listing.laws,
        strength_missing: STRENGTH_MISSING.into(),
        warnings: listing.warnings,
    };
    super::present_stored(source, content, &mut reply.laws)?;
    Ok(serde_json::to_value(reply)?)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Params {
    branch_id: String,
    family: Family,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    reads: Vec<ReadHour>,
    #[serde(default)]
    within: BTreeMap<String, Vec<HourSpan>>,
    from: Option<f64>,
    to: Option<f64>,
    width: Option<u32>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Family {
    measure: OccupationMeasure,
    rank_measure: OccupationMeasure,
    #[serde(default)]
    both: bool,
    top: Option<u32>,
    #[serde(default)]
    collaboration: bool,
}
#[derive(Deserialize)]
struct ReadHour {
    hour: i32,
}

fn measure_column(measure: OccupationMeasure) -> usize {
    match measure {
        OccupationMeasure::Resistance => 0,
        OccupationMeasure::Compliance => 1,
    }
}

/// The hours of `spans` inside `within`.
fn pieces(
    spans: &[HourSpan],
    within: &[HourSpan],
    cancelled: &AtomicBool,
) -> io::Result<Vec<HourSpan>> {
    let mut pieces = Vec::new();
    dashboard_base::ranking::for_each_piece(
        spans,
        within,
        |span| (span.first_hour, span.last_hour),
        cancelled,
        |_, first_hour, last_hour| {
            pieces.push(HourSpan {
                first_hour,
                last_hour,
            });
            Ok(())
        },
    )?;
    Ok(pieces)
}

/// A series request of the `occupation-trend` family kind (`series-summary`, `series-state` or
/// `series-next-change`) answered from the family's publication; null before it publishes the
/// branch.
pub fn trend_query(
    source: &Source,
    context: &LoadContext,
    product: &str,
    value: Value,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let params: Params = serde_json::from_value(value.clone())?;
    let branch = params
        .branch_id
        .parse()
        .map_err(|_| dashboard_core::invalid("Invalid occupation branch"))?;
    let selection = OccupationTrendParams {
        branch_id: params.branch_id.clone(),
        tags: params.tags.clone(),
        top: params.family.top.map(|count| OccupationTop {
            count,
            measure: params.family.rank_measure,
        }),
        collaboration: params.family.collaboration,
    };
    occupation_trend::OccupationTrend::validate(&selection).map_err(dashboard_core::invalid)?;
    let content = context.content.as_ref();
    let (store, key) = dashboard_base::prepared::files(source, content)?;
    let coverage = dashboard_base::prepared::coverage::product_coverage(
        source,
        content,
        branch,
        "occupation-trend",
        cancelled,
    )?;
    let Some(opened) = published::open(source, content, NAME, SCHEMA, branch)? else {
        return Ok(Value::Null);
    };
    let complete_histories = dashboard_base::products::country_history::prepared_branch_histories(
        source, branch, content, cancelled,
    )?;
    let identities_preparing = complete_histories.is_none();
    let histories = match complete_histories {
        Some(history) => history,
        None => {
            let accepted = dashboard_base::prepared::coverage::product_coverage(
                source,
                content,
                branch,
                "country-identities",
                cancelled,
            )?;
            Arc::new(dashboard_base::prepared::query::progressive::histories(
                &store,
                &key,
                branch,
                accepted.as_ref(),
                &BTreeMap::new(),
                &BTreeMap::new(),
                source.replay_ref(),
                &params.tags,
                cancelled,
            )?)
        }
    };
    let mut data = opened.read(cancelled, |snapshot, directory| {
        answer(
            snapshot,
            directory,
            product,
            &value,
            &params,
            &selection,
            &histories,
            coverage.as_ref(),
            cancelled,
        )
    })?;
    let prefix = dashboard_base::prepared::prefix::Prefix::new(
        source,
        branch,
        dashboard_base::prepared::prefix::extent(source, branch, coverage.clone())?,
        if product == "series-state" {
            params.reads.first().map(|read| read.hour)
        } else {
            params.to.map(|hour| hour.floor() as i32)
        },
    );
    data["pending"] = serde_json::to_value(prefix.pending)?;
    if identities_preparing {
        data["preparing"] = json!(["identities", "segments", "startsSegment"]);
    }
    data["coverage"] = serde_json::to_value(
        dashboard_base::prepared::query::progressive::identity_coverage(
            source,
            content,
            branch,
            coverage.as_ref(),
            !identities_preparing,
            cancelled,
        )?,
    )?;
    Ok(data)
}

/// [`trend_query`]'s reply from `snapshot`, the family's publication, and its `directory`.
#[allow(clippy::too_many_arguments)]
fn answer<P: dashboard_series::PageRead>(
    snapshot: &mut dashboard_series::Snapshot<'_, P>,
    directory: &dashboard_series::family::Directory,
    product: &str,
    value: &Value,
    params: &Params,
    selection: &OccupationTrendParams,
    histories: &dashboard_base::products::country_history::RecordedHistories,
    coverage: Option<&dashboard_base::prepared::coverage::Coverage>,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let tags = read::occupiers(directory);
    let (mut warnings, collaboration_warnings) = read::trend_warnings(directory)?;
    if params.family.collaboration {
        warnings.extend(collaboration_warnings);
    }
    let selected: Vec<String> = if params.family.top.is_some() || params.tags.is_empty() {
        tags.keys().cloned().collect()
    } else {
        dashboard_base::products::country_history::selected_aliases(histories, &params.tags)
    };
    let selected_ids = (!params.tags.is_empty() && params.family.top.is_none()).then(|| {
        dashboard_base::products::country_history::selected_identities(histories, &params.tags)
    });
    let mut within = BTreeMap::new();
    for tag in &selected {
        if let Some(selected_ids) = &selected_ids {
            let spans: Vec<_> = histories
                .countries
                .get(tag)
                .into_iter()
                .flat_map(|country| country.identities.iter())
                .filter(|identity| selected_ids.contains(&(identity.slot, identity.country_id)))
                .map(|identity| HourSpan {
                    first_hour: identity.first_hour,
                    last_hour: identity.last_hour,
                })
                .collect();
            if params.within.is_empty() {
                within.insert(tag.clone(), spans);
            } else {
                let asked = params.within.get(tag).map_or(&[][..], Vec::as_slice);
                within.insert(tag.clone(), pieces(&spans, asked, cancelled)?);
            }
        } else if let Some(spans) = params.within.get(tag) {
            within.insert(tag.clone(), spans.clone());
        }
    }
    let column = measure_column(params.family.measure);
    let rank_column = measure_column(params.family.rank_measure);
    match product {
        "series-summary" => {
            let view = series::viewport(params.from, params.to, params.width)?;
            let bounds = (view.0.floor() as i32, view.1.ceil() as i32);
            let mut lines = Vec::new();
            for tag in selected {
                check_cancel(cancelled)?;
                let Some(&actor) = tags.get(&tag) else {
                    continue;
                };
                let mut spans = within.get(&tag).cloned();
                if let Some(top) = params.family.top {
                    // A ranked line holds only the hours its country ranks within the top, of
                    // those asked for: it ends where the country leaves the selection.
                    let mut ranked = Vec::new();
                    // The summary's bins end at the viewport's last whole hour.
                    let last = view.1.floor() as i32;
                    if bounds.0 <= last {
                        snapshot.top_spans(
                            dashboard_series::Selection {
                                key: read::key(directory, actor, RANKS[rank_column])?,
                                membership: None,
                            },
                            u64::from(top),
                            dashboard_series::Span::new(bounds.0, last)?,
                            |span| {
                                ranked.push(HourSpan {
                                    first_hour: span.first,
                                    last_hour: span.last,
                                });
                                Ok(())
                            },
                        )?;
                    }
                    let ranked = match &spans {
                        Some(asked) => pieces(&ranked, asked, cancelled)?,
                        None => ranked,
                    };
                    if ranked.is_empty() {
                        continue;
                    }
                    spans = Some(ranked);
                }
                let means = read::key(directory, actor, MEANS[column])?;
                let mut summary =
                    engine::summary(snapshot, means, bounds, view, 5, spans.as_deref())?;
                let facts = read::key(directory, actor, columnar::emit::FACTS)?;
                if summary.runs.is_some() {
                    let mut raw =
                        read::fact_runs(snapshot, directory, actor, bounds.0, view.1.floor() as i32)?;
                    for run in &mut raw {
                        let canonical = read::run_at(snapshot, facts, run.first_hour)?
                            .ok_or_else(|| {
                                dashboard_core::invalid("Occupation supplying run is missing")
                            })?;
                        run.first_hour = canonical.span.first;
                        run.last_hour = canonical.span.last;
                        run.starts_segment = canonical.break_before;
                    }
                    let mut clipped = Vec::new();
                    dashboard_base::ranking::for_each_optional_piece(
                        &raw,
                        spans.as_deref(),
                        |run| (run.first_hour, run.last_hour),
                        cancelled,
                        |run, first_hour, last_hour| {
                            clipped.push(OccupationTrendRun {
                                first_hour,
                                last_hour,
                                starts_segment: run.starts_segment && run.first_hour == first_hour,
                                ..run.clone()
                            });
                            Ok(())
                        },
                    )?;
                    let history = History {
                        series: BTreeMap::from([(tag.clone(), clipped)]),
                        ..Default::default()
                    };
                    let reply = occupation_trend::reply_with_histories(
                        &params.branch_id,
                        &history,
                        &OccupationTrendParams {
                            branch_id: params.branch_id.clone(),
                            tags: vec![tag.clone()],
                            top: None,
                            collaboration: params.family.collaboration,
                        },
                        Vec::new(),
                        cancelled,
                        None,
                    )?;
                    summary.runs = Some(
                        reply
                            .series
                            .into_iter()
                            .flat_map(|line| line.runs)
                            .map(|run| {
                                let raw = if column == 0 {
                                    run.resistance_raw
                                } else {
                                    run.compliance_raw
                                };
                                Ok(dashboard_base::prepared_chart::ChartRun {
                                    first_hour: run.first_hour,
                                    last_hour: run.last_hour,
                                    value: raw.parse::<i64>().map_err(|_| {
                                        dashboard_core::invalid("Invalid occupation fixed5 value")
                                    })? as f64
                                        / 100_000.0,
                                    break_before: run.starts_segment,
                                })
                            })
                            .collect::<io::Result<_>>()?,
                    );
                }
                series::summary_budget(view.0, view.1, view.2, lines.len() + 1)?;
                let end_run = match summary
                    .buckets
                    .iter()
                    .rev()
                    .flatten()
                    .next()
                    .map(|bucket| bucket.last_hour)
                {
                    Some(end) if read::run_at(snapshot, facts, end)?.is_some() => {
                        let mut run = read::fact_at(snapshot, directory, actor, end)?;
                        run.first_hour = end;
                        run.last_hour = end;
                        run.starts_segment = false;
                        if !params.family.collaboration {
                            run.collaboration.clear();
                        }
                        Some(run)
                    }
                    _ => None,
                };
                let country = histories.countries.get(&tag);
                let identities = country.map(|country| {
                    country
                        .identities
                        .iter()
                        .filter(|identity| {
                            selected_ids.as_ref().is_none_or(|ids| {
                                ids.contains(&(identity.slot, identity.country_id))
                            })
                        })
                        .collect::<Vec<_>>()
                });
                lines.push(json!({"tag":tag,"summary":summary,"marks":null,"endRun":end_run,"segments":within.get(&tag).or_else(||country.map(|country|country.segments.as_ref())),"identities":identities}));
            }
            Ok(json!({"branchId":params.branch_id,"measure":params.family.measure,"decimals":5,"series":lines,"warnings":warnings}))
        }
        "series-next-change" => {
            let navigation: series::Navigation = serde_json::from_value(value.clone())?;
            let hour = navigation
                .origin()?
                .clamp(navigation.first_hour, navigation.last_hour)
                .floor() as i32;
            let mut actors: Vec<(u64, Option<&[HourSpan]>)> = Vec::new();
            let mut keys: Vec<(u64, Option<&[HourSpan]>)> = Vec::new();
            if let Some(top) = params.family.top {
                for rank in 0..u64::from(top).min(tags.len() as u64) {
                    let Some(position) =
                        directory.key(columnar::emit::FAMILY, POSITIONS[rank_column], Some(rank))
                    else {
                        continue;
                    };
                    keys.push((position, None));
                    if let Some(run) = read::run_at(snapshot, position, hour)? {
                        let actor = u64::try_from(run.value).map_err(|_| {
                            dashboard_core::invalid("A prepared occupation rank names no occupier")
                        })?;
                        actors.push((actor, None));
                    }
                }
            } else {
                actors.extend(selected.iter().filter_map(|tag| {
                    tags.get(tag)
                        .map(|&actor| (actor, within.get(tag).map(Vec::as_slice)))
                }));
            }
            for &(actor, spans) in &actors {
                keys.push((read::key(directory, actor, MEANS[column])?, spans));
                if params.family.both {
                    keys.push((read::key(directory, actor, MEANS[1 - column])?, spans));
                }
                if params.family.collaboration {
                    keys.push((read::key(directory, actor, columnar::emit::FACTS)?, spans));
                }
            }
            let hour = engine::navigate(snapshot, &keys, &navigation)?;
            Ok(json!({"hour":hour}))
        }
        "series-state" => {
            let [asked] = params.reads.as_slice() else {
                return Err(dashboard_core::invalid(
                    "Occupation state needs one reached hour",
                ));
            };
            let mut history = History::default();
            for tag in selected {
                let Some(&actor) = tags.get(&tag) else {
                    continue;
                };
                if within.get(&tag).is_some_and(|spans| {
                    !spans
                        .iter()
                        .any(|span| span.first_hour <= asked.hour && span.last_hour >= asked.hour)
                }) {
                    continue;
                }
                if coverage.is_some_and(|coverage| asked.hour <= coverage.last_hour) {
                    let runs = read::fact_runs(snapshot, directory, actor, asked.hour, asked.hour)?;
                    if !runs.is_empty() {
                        history.series.insert(tag.clone(), runs);
                    }
                }
                if let Some(country) = histories.countries.get(&tag) {
                    history
                        .segments
                        .insert(tag, country.segments.as_ref().clone());
                }
            }
            Ok(serde_json::to_value(
                occupation_trend::reply_with_histories(
                    &params.branch_id,
                    &history,
                    selection,
                    warnings,
                    cancelled,
                    Some(histories),
                )?,
            )?)
        }
        _ => Err(dashboard_core::invalid("Unknown occupation series query")),
    }
}
