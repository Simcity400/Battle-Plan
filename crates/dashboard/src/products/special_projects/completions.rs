//! Lean all-country completion scan shared with the project's history builder.
//! Progress runs stay country-local; the feed caches only actual completions.

use super::*;
use replay::{container::Replay, frame::FrameDecoder};

#[cfg(test)]
const CACHED_BRANCHES: usize = 4;

#[cfg(test)]
#[derive(Default)]
struct WorldBuilder {
    /// The recording's format, which dates its daily tables
    /// ([`crate::metadata::slot_read`]).
    format: replay::format::FormatVersion,
    countries: BTreeMap<(u32, u32), Builder>,
    campaign_start: Option<i32>,
    warnings: BTreeMap<String, usize>,
    views: Option<(i32, [ViewIdentity; 4])>,
    #[cfg(test)]
    projected_frames: usize,
}

#[cfg(test)]
impl WorldBuilder {
    #[cfg(test)]
    fn observe(&mut self, frame: &DecodedFrame) {
        const VIEWS: [TableId; 4] = [
            TableId::Countries,
            TableId::CountryLinks,
            TableId::Projects,
            TableId::ProjectHistory,
        ];
        let unchanged = self.views.as_ref().is_some_and(|(previous, views)| {
            previous.checked_add(1) == Some(frame.hour)
                && views
                    .iter()
                    .zip(VIEWS)
                    .all(|(view, table)| view.is(&frame.tables[table.index()]))
        });
        // A first successful read still admits a previously unadmitted row set.
        // Otherwise unchanged compact rows need only continuity, not new maps.
        let read = slot_read(self.format, &frame.metrics, TableId::Projects, frame.hour);
        if unchanged
            && (!read
                || self
                    .countries
                    .values()
                    .all(|builder| builder.admitted || builder.last.is_none()))
        {
            for builder in self.countries.values_mut() {
                if let Some((_, previous)) = builder.last.as_mut()
                    && previous.checked_add(1) == Some(frame.hour)
                {
                    *previous = frame.hour;
                }
            }
            self.views.as_mut().expect("unchanged views exist").0 = frame.hour;
            return;
        }
        self.views = Some((
            frame.hour,
            VIEWS.map(|table| frame.tables[table.index()].identity()),
        ));
        #[cfg(test)]
        {
            self.projected_frames += 1;
        }
        let (Ok(countries), Ok(links)) = (
            frame.complete::<tables::Countries>(),
            frame.complete::<tables::CountryLinks>(),
        ) else {
            for builder in self.countries.values_mut() {
                builder.observe_rows(frame, "", None, None, None, None);
            }
            return;
        };
        let identities = tagged_countries(countries, links);
        let mut projects = BTreeMap::<u32, Vec<&ProjectRow>>::new();
        let project_rows = frame.complete::<tables::Projects>().ok();
        if let Some(rows) = project_rows {
            for project in rows {
                if let Some(slot) = link_slot(links, project.owner) {
                    projects.entry(slot).or_default().push(project);
                } else {
                    *self
                        .warnings
                        .entry(prepared::completion_stream::UNMAPPED_PROJECT_OWNER.into())
                        .or_default() += 1;
                }
            }
        }
        let mut histories = BTreeMap::<u32, Vec<&ProjectHistoryRow>>::new();
        let history_rows = frame.complete::<tables::ProjectHistory>().ok();
        if let Some(rows) = history_rows {
            for event in rows {
                if let Some(slot) = link_slot(links, event.owner) {
                    histories.entry(slot).or_default().push(event);
                } else {
                    *self
                        .warnings
                        .entry(prepared::completion_stream::UNMAPPED_HISTORY_OWNER.into())
                        .or_default() += 1;
                }
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        for (tag, identity) in identities {
            let Ok(tag) = std::str::from_utf8(tag) else {
                *self
                    .warnings
                    .entry("Project country tag is non-UTF-8.".into())
                    .or_default() += 1;
                continue;
            };
            let Some(identity) = identity.found().map(|(country, slot)| (slot, country.id)) else {
                continue;
            };
            seen.insert(identity);
            let builder = self.countries.entry(identity).or_insert_with(|| Builder {
                format: self.format,
                compact: true,
                campaign_start: self.campaign_start,
                ..Default::default()
            });
            let slot = identity.0;
            builder.observe_rows(
                frame,
                tag,
                Some(identity),
                project_rows.map(|_| projects.get(&slot).map_or(&[][..], Vec::as_slice)),
                history_rows.map(|_| histories.get(&slot).map_or(&[][..], Vec::as_slice)),
                None,
            );
        }
        for (identity, builder) in &mut self.countries {
            if !seen.contains(identity) {
                builder.observe_rows(frame, "", None, None, None, None);
            }
        }
    }
}

#[cfg(test)]
fn scan_completions(
    replay: &Replay,
    scan: &history::BranchScan,
    branch: u64,
    content: Option<&mut Content>,
    cancelled: &AtomicBool,
) -> io::Result<ProjectCompletions> {
    let mut world = WorldBuilder {
        format: scan.format,
        campaign_start: scan.campaign_start,
        ..Default::default()
    };
    for span in &scan.spans {
        crate::metadata::check_cancel(cancelled)?;
        let unresolved = span.inputs.unresolved_actors();
        if unresolved > 0 {
            *world
                .warnings
                .entry("Project country identity could not be resolved.".into())
                .or_default() += unresolved;
        }
        let mut frame = span.inputs.world_frame(branch);
        for hour in span.first_hour..=span.last_hour {
            crate::metadata::check_cancel(cancelled)?;
            frame.hour = hour;
            frame.metrics.table_ns[TableId::Projects.index()] =
                u64::from(span.reads.binary_search(&hour).is_ok());
            world.observe(&frame);
        }
    }
    finish(world, replay, branch, content, cancelled)
}

#[cfg(test)]
fn finish(
    world: WorldBuilder,
    replay: &Replay,
    branch: u64,
    mut content: Option<&mut Content>,
    cancelled: &AtomicBool,
) -> io::Result<ProjectCompletions> {
    let available = world
        .countries
        .values()
        .any(|builder| !builder.available.is_empty());
    let mut observations = Vec::new();
    let mut counts = world.warnings;
    for builder in world.countries.into_values() {
        for (reason, count) in builder.warnings {
            *counts.entry(structural_reason(&reason)).or_default() += count;
        }
        observations.extend(builder.completions.into_values());
    }
    let presented = present_observations(
        replay,
        branch,
        observations
            .into_iter()
            .enumerate()
            .map(|(id, row)| (id as u64, row))
            .collect(),
        content.as_deref_mut(),
        cancelled,
    )?;
    let mut events = Vec::new();
    for row in presented {
        if let Some(event) = row.completion {
            events.push(event);
        }
        for (reason, count) in row.warnings {
            *counts.entry(reason).or_default() += count;
        }
    }
    let warnings = content.as_ref().map_or_else(Vec::new, |content| {
        let mut warnings = content.definitions.warnings.clone();
        warnings.extend(content.catalogue.verification.iter().cloned());
        warnings
    });
    let texts = take_refusals(content.as_deref());
    let refusals = refusal_totals(counts, &texts);
    if let Some(content) = content.as_ref() {
        content
            .refusals
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .merge(&texts);
    }
    Ok(ProjectCompletions {
        events,
        warnings,
        refusals,
        unavailable: (!available).then(|| {
            "Projects was unavailable, incomplete, or never read for any country.".to_owned()
        }),
    })
}

pub(super) fn refusal_totals(
    mut counts: BTreeMap<String, usize>,
    texts: &catalogue::Refusals,
) -> Vec<(String, usize)> {
    for (reason, count) in texts.counted_diagnostic_lines("Project texts") {
        *counts.entry(reason).or_default() += count;
    }
    counts.into_iter().collect()
}

/// Transient presentation work; raw observations never enter prepared state.
pub(super) struct PresentedCompletion {
    pub id: u64,
    pub observation: CompletionObservation,
    pub completion: Option<ProjectCompletion>,
    pub warnings: BTreeMap<String, usize>,
    pub refusals: catalogue::Refusals,
}

/// Present newly recognised changes at their native hours, decoding each selected
/// block once. The recording oracle and preparation share this presenter.
pub(super) fn present_observations(
    replay: &Replay,
    branch: u64,
    observations: Vec<(u64, CompletionObservation)>,
    mut content: Option<&mut Content>,
    cancelled: &AtomicBool,
) -> io::Result<Vec<PresentedCompletion>> {
    let mut pending = BTreeMap::<i32, Vec<(u64, CompletionObservation)>>::new();
    for (id, observation) in observations {
        pending
            .entry(observation.date.hour)
            .or_default()
            .push((id, observation));
    }
    if pending.is_empty() {
        return Ok(Vec::new());
    }
    let scoped = content.as_ref().is_some_and(|content| {
        scripted_names(&content.definitions, &content.catalogue.localisation)
    });
    let mut selected = vec![
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::LexicalTokens,
        TableId::ProjectPrograms,
        TableId::Characters,
        TableId::CharacterPortraits,
        TableId::CountryAppearance,
    ];
    if scoped {
        selected.push(TableId::Projects);
        selected.extend_from_slice(crate::triggers::RECORDED_TABLES);
        selected.sort_unstable();
        selected.dedup();
    }
    let blocks: Vec<_> = replay
        .index
        .iter()
        .filter(|block| {
            block.branch == branch
                && pending
                    .range(block.first_hour..=block.last_hour)
                    .next()
                    .is_some()
        })
        .collect();
    let saved_names = if scoped {
        content
            .as_deref_mut()
            .map(|content| std::mem::take(&mut content.scoped))
    } else {
        None
    };
    let mut names = history::NameScan::default();
    let mut aggregate = take_refusals(content.as_deref());
    let result = (|| {
        let mut result = Vec::new();
        crate::blocks::scan_blocks(replay, blocks, &selected, cancelled, |blocks| {
            for (block, frames) in blocks {
                crate::metadata::check_cancel(cancelled)?;
                let Some((&last_needed, _)) = pending
                    .range(block.first_hour..=block.last_hour)
                    .next_back()
                else {
                    continue;
                };
                let Ok(frames) = frames else { continue };
                let mut decoder = FrameDecoder::for_tables(&selected);
                for frame in frames {
                    if frame.hour > last_needed {
                        break;
                    }
                    crate::metadata::check_cancel(cancelled)?;
                    let Ok(frame) = decoder.advance(&frame) else {
                        break;
                    };
                    if validate_campaign(frame, replay).is_err() {
                        break;
                    }
                    if scoped && let Some(content) = content.as_deref_mut() {
                        names.observe(
                            frame,
                            &history::Actors::read(frame),
                            slot_read(replay.format, &frame.metrics, TableId::Projects, frame.hour),
                            content,
                            cancelled,
                        )?;
                        content.scoped.clear();
                        content
                            .scoped
                            .push((branch, Arc::new(names.take_current())));
                    }
                    let Some(observations) = pending.remove(&frame.hour) else {
                        continue;
                    };
                    let identities = frame
                        .complete::<tables::Countries>()
                        .ok()
                        .zip(frame.complete::<tables::CountryLinks>().ok())
                        .map(|(countries, links)| {
                            tagged_countries(countries, links)
                                .into_iter()
                                .filter_map(|(tag, lookup)| {
                                    lookup
                                        .found()
                                        .map(|(country, slot)| ((slot, country.id), tag))
                                })
                                .collect::<BTreeMap<_, _>>()
                        })
                        .unwrap_or_default();
                    for (id, observation) in observations {
                        let mut warnings = BTreeMap::new();
                        let completion = match identities.get(&observation.identity) {
                            None => {
                                warnings.insert("Project country identity could not be resolved at its completion hour.".into(), 1);
                                None
                            }
                            Some(tag) => {
                                match std::str::from_utf8(tag) {
                                    Err(_) => {
                                        warnings.insert("Project country tag at its completion hour is non-UTF-8.".into(), 1);
                                        None
                                    }
                                    Ok(tag) => Some(present_completion(
                                        tag.to_owned(),
                                        observation.clone(),
                                        frame,
                                        content.as_deref_mut(),
                                        &mut warnings,
                                    )),
                                }
                            }
                        };
                        let refusals = take_refusals(content.as_deref());
                        aggregate.merge(&refusals);
                        result.push(PresentedCompletion {
                            id,
                            observation,
                            completion,
                            warnings,
                            refusals,
                        });
                    }
                }
            }
            Ok(())
        })?;
        for (id, observation) in pending.into_values().flatten() {
            result.push(PresentedCompletion {
                id,
                observation,
                completion: None,
                warnings: BTreeMap::from([("Project completion hour was not recorded.".into(), 1)]),
                refusals: Default::default(),
            });
        }
        Ok(result)
    })();
    if let Some(content) = content {
        content
            .refusals
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .merge(&aggregate);
        if let Some(saved) = saved_names {
            content.scoped = saved;
        }
    }
    result
}

/// Country-prefixed project text follows the same native selector as the topic.
/// Dollar aliases are expanded before deciding whether country frame inputs are needed.
pub(super) fn scripted_names(
    definitions: &Catalog,
    localisation: &installed_content::localisation::Localisation,
) -> bool {
    let keys: std::collections::BTreeSet<_> = definitions
        .projects
        .iter()
        .map(|(token, project)| {
            project
                .name
                .as_deref()
                .filter(|name| !name.is_empty())
                .unwrap_or(token)
        })
        .collect();
    localisation.iter().any(|(candidate, _)| {
        (keys.contains(candidate)
            || keys.iter().any(|key| {
                candidate
                    .strip_suffix(*key)
                    .is_some_and(|prefix| prefix.ends_with('_'))
            }))
            && needs_scope(localisation, candidate)
    })
}

pub(super) fn structural_reason(reason: &str) -> String {
    if let Some((project, offset)) = reason
        .strip_prefix("Project ")
        .and_then(|rest| rest.split_once(" has unsupported native state offset "))
        && project.parse::<u32>().is_ok()
    {
        // Local handles vary by country; the unsupported native offset is the
        // stable form that must remain named. Content-token reasons stay exact.
        format!("Project has unsupported native state offset {offset}")
    } else {
        reason.to_owned()
    }
}

pub(super) fn present_completion(
    tag: String,
    observation: CompletionObservation,
    frame: &DecodedFrame,
    mut content: Option<&mut Content>,
    warnings: &mut BTreeMap<String, usize>,
) -> ProjectCompletion {
    let project = &observation.project;
    let mut event = ProjectCompletion {
        tag: tag.clone(),
        date: observation.date,
        observed_hour: observation.observed_hour,
        definition_id: project.definition_id,
        item_token: None,
        name: None,
        icon: None,
        icon_missing: None,
    };
    event.item_token = frame
        .complete::<tables::LexicalTokens>()
        .ok()
        .and_then(|tokens| {
            tokens
                .binary_search_by_key(&project.definition_id, |row| row.token)
                .ok()
                .and_then(|index| std::str::from_utf8(&tokens[index].literal).ok())
                .map(str::to_owned)
        });
    let mut presentation = Builder::default();
    let has_content = content.is_some();
    let index = presentation.presentation(
        frame,
        project,
        &tag,
        observation.identity.0,
        None,
        content.as_deref_mut(),
    );
    let info = &presentation.presentations[index as usize];
    event.name = event
        .item_token
        .as_ref()
        .filter(|_| has_content)
        .map(|_| info.name.clone());
    event.icon = info.icon.clone();
    event.icon_missing = info.icon_missing.clone();
    if let Some(content) = content.as_ref()
        && let Some((_, names)) = content
            .scoped
            .iter()
            .find(|(seen, _)| *seen == frame.branch)
    {
        for reason in names.refusals_at(
            &tag,
            observation.identity.0,
            project.definition_id,
            frame.hour,
        ) {
            *warnings.entry(reason.clone()).or_default() += 1;
        }
    }
    for (reason, count) in presentation.warnings {
        if event.icon_missing.as_deref() != Some(reason.as_str()) {
            *warnings.entry(structural_reason(&reason)).or_default() += count;
        }
    }
    event
}

#[cfg(test)]
pub(crate) fn completion_events(
    source: &Source,
    branch: u64,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Arc<ProjectCompletions>> {
    let politics = context
        .content()
        .and_then(|options| politics_content(source, options));
    let state = source.state::<Cache>();
    let catalogue = politics
        .as_ref()
        .ok()
        .map(|content| Arc::clone(&content.catalogue));
    let same_content = |cached: &Option<Arc<Catalogue>>| match (cached, &catalogue) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    };
    let held = {
        let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
        if same_content(&cache.catalogue) {
            if let Some(index) = cache
                .completions
                .iter()
                .position(|(seen, _)| *seen == branch)
            {
                let entry = cache
                    .completions
                    .remove(index)
                    .expect("cached branch exists");
                let events = Arc::clone(&entry.1);
                cache.completions.push_back(entry);
                return Ok(events);
            }
            Some((cache.content.clone(), cache.content_error.clone()))
        } else {
            None
        }
    };
    // Parsing and scanning happen outside the shared topic cache lock. A local
    // content projection keeps its mutable icon memo private to this build.
    let mut prepared = Cache::default();
    let (mut content, content_error) = match held {
        Some(held) => held,
        None => {
            prepare_cache(&mut prepared, &politics);
            (prepared.content.take(), prepared.content_error.take())
        }
    };
    let scan = history::cached(source, branch, cancelled)?;
    let mut events = scan_completions(
        source.replay_ref(),
        &scan,
        branch,
        content.as_mut(),
        cancelled,
    )?;
    if let Some(reason) = &content_error {
        events.refusals.push((reason.clone(), 1));
    }
    if let Err(reason) = politics {
        events.refusals.push((
            format!("Project content, names and art unavailable: {reason}"),
            1,
        ));
    }
    let events = Arc::new(events);
    let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
    if !same_content(&cache.catalogue) {
        #[cfg(test)]
        cache.replies.clear();
        cache.completions.clear();
        cache.catalogue = catalogue;
        cache.content = content;
        cache.content_error = content_error;
    }
    cache.completions.retain(|(seen, _)| *seen != branch);
    cache.completions.push_back((branch, Arc::clone(&events)));
    // A cache budget, never a country limit or a truncation of branch facts.
    while cache.completions.len() > CACHED_BRANCHES {
        cache.completions.pop_front();
    }
    Ok(events)
}

#[cfg(test)]
mod tests;
