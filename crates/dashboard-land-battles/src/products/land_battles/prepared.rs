//! Prepared block records reuse the census, peak helpers and reply builders.
use super::*;
use dashboard_base::prepared::series;
use serde_json::Value;

impl Census {
    pub fn new(format: replay::format::FormatVersion) -> Self {
        Self {
            format,
            ..Self::default()
        }
    }
    pub fn configure(&mut self, source: &Source, context: &LoadContext) {
        if self.configured {
            return;
        }
        self.configured = true;
        self.format = source.replay_ref().format;
        let (catalogue, warning) = battle_content(source, context);
        self.catalogue = catalogue;
        self.content_warnings = warning.into_iter().collect();
    }
    pub(super) fn window_records(&self, window: &BattleWindow) -> Self {
        let first = window.first_hour;
        let last = window.last_hour;
        let episodes = self
            .episodes
            .iter()
            .filter_map(|episode| {
                let episode = self.effective_episode(episode);
                let end_in_window = episode
                    .end
                    .as_ref()
                    .is_some_and(|(hour, _)| *hour >= first && *hour <= last);
                let mut runs: Vec<_> = episode
                    .runs
                    .iter()
                    .filter(|run| run.first_hour <= last && run.last_hour >= first)
                    .map(|run| run.clipped(run.first_hour.max(first), run.last_hour.min(last)))
                    .collect();
                if runs.is_empty() && end_in_window {
                    let run = episode.runs.last().expect("observed episode");
                    runs.push(run.clipped(run.last_hour, run.last_hour));
                }
                if runs.is_empty() {
                    return None;
                }
                let mut row = episode.as_ref().clone();
                row.runs = runs;
                let loss_first = first.min(row.runs[0].first_hour);
                row.losses = episode
                    .loss_at(loss_first)
                    .into_iter()
                    .chain(
                        episode
                            .losses
                            .iter()
                            .filter(|read| read.hour > loss_first && read.hour <= last),
                    )
                    .cloned()
                    .collect();
                row.loss_inputs = None;
                row.prior_peaks = std::array::from_fn(|index| {
                    first.checked_sub(1).and_then(|hour| {
                        if index == 0 {
                            episode.peak_before(hour)
                        } else {
                            episode.side_peak_before(hour, index - 1)
                        }
                    })
                });
                Some(row)
            })
            .collect();
        Self {
            episodes,
            observed: self
                .observed_spans()
                .filter(|(from, to)| *from <= last && *to >= first)
                .chain(self.observed_spans().rev().find(|(_, to)| *to < first))
                .collect(),
            warnings: self.warning_counts(first, last),
            ..Self::default()
        }
    }
    pub fn publish_block(
        &mut self,
        writer: &mut store::Writer,
        replay: &Replay,
        branch: u64,
        ordinal: usize,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let mut own = std::mem::take(&mut self.refusals);
        let published = dashboard_text::catalogue::own_refusals(&mut own, || {
            self.publish_own_block(writer, replay, branch, ordinal, cancelled)
        });
        self.refusals = own;
        published
    }
    fn publish_own_block(
        &mut self,
        writer: &mut store::Writer,
        replay: &Replay,
        branch: u64,
        ordinal: usize,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        check_cancel(cancelled)?;
        self.materialize()?;
        let windows = windows(replay, branch);
        let window = &windows[ordinal];
        let boundary = if ordinal == 0 {
            i32::MIN
        } else {
            windows[ordinal - 1].last_hour.saturating_add(1)
        };
        self.windows.event(0, boundary, &(ordinal as u64))?;
        self.windows.through(window.last_hour);
        self.windows.flush(
            writer,
            &format!("land-battle-windows-{branch}"),
            0,
            cancelled,
        )?;

        let mut changed = std::collections::BTreeSet::from([ordinal]);
        for &at in &self.dirty {
            check_cancel(cancelled)?;
            let episode = &self.episodes[at];
            // A delayed finalizer can add terminal facts to an earlier block.
            if let Some(result) = &episode.result {
                for hour in [
                    result.hour,
                    episode.runs.last().expect("observed episode").last_hour,
                ] {
                    let index = windows.partition_point(|window| window.last_hour < hour);
                    if index <= ordinal && index < windows.len() {
                        changed.insert(index);
                    }
                }
            }
            let mut used = Numbered::default();
            let battle = episode.course(&mut used);
            let reply = LandBattleCourseReply {
                battle,
                designs: self.reply_designs(used),
                content_warnings: Vec::new(),
            };
            let next = self.pages.len() as u64;
            let page = *self.pages.entry(episode.id.clone()).or_insert(next);
            series::write_reply(
                writer,
                &format!("land-battle-course-{branch}"),
                page,
                &reply,
            )?;
        }
        let mut ranking_changed = changed.clone();
        for &at in &self.dirty {
            check_cancel(cancelled)?;
            let episode = &self.episodes[at];
            for hour in episode
                .end
                .as_ref()
                .map(|(hour, _)| *hour)
                .into_iter()
                .chain(episode.result.as_ref().map(|result| result.hour))
            {
                let index = windows.partition_point(|window| window.last_hour < hour);
                if index <= ordinal && index < windows.len() {
                    ranking_changed.insert(index);
                }
            }
        }
        for index in ranking_changed {
            check_cancel(cancelled)?;
            let first = if index == 0 {
                self.episodes
                    .iter()
                    .map(|episode| episode.start.hour)
                    .min()
                    .unwrap_or(windows[0].first_hour)
                    .min(windows[0].first_hour)
            } else {
                windows[index - 1]
                    .last_hour
                    .checked_add(1)
                    .unwrap_or(windows[index].first_hour)
            };
            let interval = BattleWindow {
                first_hour: first,
                last_hour: windows[index].last_hour,
            };
            series::write_reply(
                writer,
                &format!("land-battle-ranking-{branch}"),
                index as u64,
                &self.window_records(&interval),
            )?;
        }
        for index in changed {
            check_cancel(cancelled)?;
            let window = &windows[index];
            series::write_reply(
                writer,
                &format!("land-battles-{branch}"),
                index as u64,
                &self.window_records(window).reply_window(index, window)?,
            )?;
        }
        self.dirty.clear();
        self.reads
            .flush(writer, &format!("land-battle-reads-{branch}"), 0, cancelled)?;
        series::write_catalogue(writer, "land-battle-keys", branch, &self.pages, &())?;
        series::write_reply(
            writer,
            &format!("land-battle-availability-{branch}"),
            0,
            &self.available,
        )?;
        series::write_reply(
            writer,
            &format!("land-battle-warnings-{branch}"),
            0,
            &self.content_warnings(),
        )
    }
}

/// Committed log pages remain useful while a newest-window proof is pending.
pub fn prefix_query(
    source: &Source,
    product: &str,
    params: &Value,
    context: &LoadContext,
    store: &store::Store,
    key: &store::SourceKey,
    cancelled: &AtomicBool,
) -> io::Result<Option<Value>> {
    let id = params["branchId"]
        .as_str()
        .ok_or_else(|| dashboard_core::invalid("Invalid battle branch"))?;
    let branch = branch_id(source, id)?;
    let prefix = dashboard_base::prepared::prefix::read_or_empty(
        source,
        context.content.as_ref(),
        product,
        branch,
        None,
        cancelled,
    )?;
    let windows = windows(source.replay_ref(), branch);
    let mut data = match product {
        "land-battles-manifest" => {
            let available = series::read_reply::<bool>(
                store,
                key,
                &format!("land-battle-availability-{branch}"),
                0,
            )?
            .unwrap_or(false);
            serde_json::to_value(LandBattlesManifestReply {
                windows,
                available,
                unavailable: None,
                content_warnings: Vec::new(),
            })?
        }
        "land-battles" => {
            let params: LandBattlesParams = serde_json::from_value(params.clone())?;
            LandBattles::validate(&params).map_err(dashboard_core::invalid)?;
            let window = windows
                .get(params.window)
                .ok_or_else(|| dashboard_core::invalid("Battle window not in this recording"))?;
            let reply = if window.last_hour <= prefix.coverage.last_hour {
                series::read_reply::<LandBattlesReply>(
                    store,
                    key,
                    &format!("land-battles-{branch}"),
                    params.window as u64,
                )?
            } else {
                None
            };
            serde_json::to_value(reply.unwrap_or_else(|| LandBattlesReply {
                window: params.window,
                interval: window.clone(),
                rows: Vec::new(),
                observed: Vec::new(),
                reasons: Vec::new(),
                warnings: Vec::new(),
                content_warnings: Vec::new(),
            }))?
        }
        _ => return Ok(None),
    };
    // Later terminal reads can repair earlier battle rows.
    prefix.mark_revisable(source, branch, &mut data)?;
    data["warningsComplete"] = Value::Bool(false);
    Ok(Some(data))
}

pub fn query(
    source: &Source,
    product: &str,
    params: Value,
    store: &store::Store,
    key: &store::SourceKey,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let id = params
        .get("branchId")
        .and_then(Value::as_str)
        .ok_or_else(|| dashboard_core::invalid("Invalid battle branch"))?;
    let branch = branch_id(source, id)?;
    match product {
        "land-battles-manifest" => {
            let params: LandBattlesManifestParams = serde_json::from_value(params)?;
            LandBattlesManifest::validate(&params).map_err(dashboard_core::invalid)?;
            let available: bool =
                series::read_reply(store, key, &format!("land-battle-availability-{branch}"), 0)?
                    .ok_or_else(|| {
                    dashboard_core::invalid("Prepared land battle availability is missing")
                })?;
            let windows = windows(source.replay_ref(), branch);
            serde_json::to_value(bounded(LandBattlesManifestReply {
                windows,
                available,
                unavailable: (!available).then(|| "Not in this recording".into()),
                content_warnings: Vec::new(),
            })?)
            .map_err(Into::into)
        }
        "land-battle-course" => {
            let warnings: Vec<String> =
                series::read_reply(store, key, &format!("land-battle-warnings-{branch}"), 0)?
                    .ok_or_else(|| {
                        dashboard_core::invalid("Prepared land battle diagnostics are missing")
                    })?;
            let params: LandBattleCourseParams = serde_json::from_value(params)?;
            LandBattleCourses::validate(&params).map_err(dashboard_core::invalid)?;
            let (keys, ()) = series::read_catalogue::<()>(store, key, "land-battle-keys", branch)?
                .ok_or_else(|| dashboard_core::invalid("Prepared battle catalogue is missing"))?;
            let page = keys
                .get(&params.id)
                .copied()
                .ok_or_else(|| dashboard_core::invalid("Battle not in this recording branch"))?;
            let mut reply: LandBattleCourseReply =
                series::read_reply(store, key, &format!("land-battle-course-{branch}"), page)?
                    .ok_or_else(|| {
                        dashboard_core::invalid("Prepared land battle course is missing")
                    })?;
            reply.content_warnings = warnings;
            serde_json::to_value(bounded(reply)?).map_err(Into::into)
        }
        "land-battles" => {
            let warnings: Vec<String> =
                series::read_reply(store, key, &format!("land-battle-warnings-{branch}"), 0)?
                    .ok_or_else(|| {
                        dashboard_core::invalid("Prepared land battle diagnostics are missing")
                    })?;
            let params: LandBattlesParams = serde_json::from_value(params)?;
            LandBattles::validate(&params).map_err(dashboard_core::invalid)?;
            let mut reply: LandBattlesReply = series::read_reply(
                store,
                key,
                &format!("land-battles-{branch}"),
                params.window as u64,
            )?
            .ok_or_else(|| dashboard_core::invalid("Battle window not in this recording"))?;
            reply.content_warnings = warnings;
            serde_json::to_value(bounded(reply)?).map_err(Into::into)
        }
        "land-battles-ranking" => {
            let warnings: Vec<String> =
                series::read_reply(store, key, &format!("land-battle-warnings-{branch}"), 0)?
                    .ok_or_else(|| {
                        dashboard_core::invalid("Prepared land battle diagnostics are missing")
                    })?;
            let params: LandBattlesRankingParams = serde_json::from_value(params)?;
            LandBattlesRanking::validate(&params).map_err(dashboard_core::invalid)?;
            let mut census = Census::default();
            let mut episodes = HashMap::new();
            let mut index = series::Reader::new(
                store,
                key,
                format!("land-battle-windows-{branch}"),
                cancelled,
            )?;
            let first = index
                .latest_event(0, params.first_hour)?
                .map(|(_, value)| serde_json::from_value::<u64>(value))
                .transpose()?
                .unwrap_or(0);
            let last = index
                .latest_event(0, params.last_hour)?
                .map(|(_, value)| serde_json::from_value::<u64>(value))
                .transpose()?
                .unwrap_or(0);
            for index in first..=last {
                check_cancel(cancelled)?;
                let page: Census = series::read_reply(
                    store,
                    key,
                    &format!("land-battle-ranking-{branch}"),
                    index,
                )?
                .ok_or_else(|| dashboard_core::invalid("Prepared land battle window is missing"))?;
                for episode in page.episodes {
                    episodes.insert(episode.id.clone(), episode);
                }
                census.warnings.extend(page.warnings);
            }
            census.episodes = episodes.into_values().collect();
            let mut reads =
                series::Reader::new(store, key, format!("land-battle-reads-{branch}"), cancelled)?;
            census.observed = reads
                .latest_event(0, params.last_hour)?
                .map(|(hour, _)| (hour, hour))
                .into_iter()
                .collect();
            census.content_warnings = warnings;
            serde_json::to_value(census.ranked(&params, cancelled)?).map_err(Into::into)
        }
        _ => unreachable!(),
    }
}
