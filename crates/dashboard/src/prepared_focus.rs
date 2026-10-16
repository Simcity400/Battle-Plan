//! Sequential focus preparation and requested-page reads. Interactive reads never decode history.
use crate::products::{
    ContentOptions, artwork,
    national_focus::{completed_for_owner, date_completions},
};
use crate::{
    Artwork, FocusEntry, FocusOrder, Source,
    metadata::{CountryKeys, check_cancel},
    prepared::{frame_changes::FrameChanges, series::Accumulator},
};
use replay::{
    frame::DecodedFrame,
    rows::{TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, atomic::AtomicBool},
};
use ts_rs::TS;

/// Row-page size limits a read, never the number of countries or completed focuses.
const PAGE_ROWS: usize = 128;

// Event/query rows keep their schema. Continuations require the active/series
// fields below; an incompatible continuation is rebuilt by the pipeline.
const STATE_SCHEMA: u32 = 5;

#[derive(Default, Clone, Debug, PartialEq, Eq)]
struct StateSchema;
impl Serialize for StateSchema {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u32(STATE_SCHEMA)
    }
}
impl<'de> Deserialize<'de> for StateSchema {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let schema = u32::deserialize(deserializer)?;
        if schema != STATE_SCHEMA {
            return Err(serde::de::Error::custom(
                "Unsupported prepared focus table/schema; rebuild focus pages",
            ));
        }
        Ok(Self)
    }
}

fn complete_order(rows: &[replay::rows::CompletedFocusRow], count: i32) -> bool {
    usize::try_from(count).ok() == Some(rows.len())
        && rows.iter().enumerate().all(|(index, row)| {
            usize::try_from(row.ordinal).ok() == Some(index) && !row.token.is_empty()
        })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Observation {
    pub hour: i32,
    pub order_first_hour: i32,
    pub tag: String,
    pub slot: u32,
    pub country_id: u32,
    pub order: FocusOrder,
    pub progress_raw: String,
    pub daily_gain_raw: Option<String>,
}
impl Observation {
    fn values(&self) -> io::Result<(i128, i128)> {
        let parse = |raw: &str| {
            raw.parse::<i128>()
                .map_err(|error| crate::invalid(error.to_string()))
        };
        Ok((
            parse(&self.progress_raw)?,
            parse(self.daily_gain_raw.as_deref().unwrap_or("0"))?,
        ))
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Previous {
    slot: u32,
    country_id: u32,
    completed_owner: u32,
    order: FocusOrder,
    order_first_hour: i32,
    progress: ProgressBaseline,
}

#[derive(Clone, Serialize, Deserialize)]
struct ProgressBaseline {
    first: i32,
    last: i32,
    raw: i64,
    previous: Option<(i64, i32, Option<String>)>,
}

/// Only the most recent completion list per tag is needed to date new observations.
/// Missing reads do not supply data; they retain attribution exactly as focus history does.
#[derive(Default, Clone, Serialize, Deserialize)]
pub struct FocusAccumulator {
    schema: StateSchema,
    branch: Option<u64>,
    hour: Option<i32>,
    previous: BTreeMap<String, Previous>,
    active: BTreeSet<String>,
    #[serde(skip)]
    keys: Option<CountryKeys>,
    #[serde(skip)]
    reusable_inputs: bool,
    #[serde(skip)]
    native: NativeFocus,
    #[serde(skip)]
    removed: Vec<String>,
    #[cfg(test)]
    #[serde(skip)]
    work: crate::prepared::observer_tests::Work,
    #[cfg(test)]
    #[serde(skip)]
    test_changes: Option<Arc<std::sync::Mutex<crate::prepared::frame_changes::FrameChangeTracker>>>,
}

/// The current operand joins, refreshed from A3 once for each changed source.
#[derive(Default, Clone)]
struct NativeFocus {
    countries: Option<Vec<replay::rows::CountryRow>>,
    links: Option<Vec<replay::rows::CountryLinkRow>>,
    focuses: Option<Vec<replay::rows::FocusRow>>,
    completed: Option<Vec<replay::rows::CompletedFocusRow>>,
    by_slot: BTreeMap<u32, replay::rows::FocusRow>,
    country_slots: BTreeMap<u32, Vec<usize>>,
    completion_slots: BTreeMap<u32, Vec<u32>>,
    unique: bool,
}
impl NativeFocus {
    fn refresh(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        countries_changed: bool,
    ) -> BTreeSet<usize> {
        let mut dirty = BTreeSet::new();
        let changed = |table: TableId| changes.rows_changed[table.index()];
        if countries_changed || self.countries.is_none() && changed(TableId::Countries) {
            self.countries = frame
                .complete::<tables::Countries>()
                .ok()
                .map(|rows| rows.to_vec());
            let mut tags = BTreeSet::new();
            self.unique = self
                .countries
                .as_ref()
                .is_some_and(|rows| rows.iter().all(|row| tags.insert(row.tag.as_slice())));
        }
        if changed(TableId::CountryLinks) {
            self.links = frame
                .complete::<tables::CountryLinks>()
                .ok()
                .map(|rows| rows.to_vec());
        }
        if countries_changed || changed(TableId::CountryLinks) {
            self.country_slots.clear();
            if let Some((countries, links)) = self.countries.as_ref().zip(self.links.as_ref()) {
                for (index, country) in countries.iter().enumerate() {
                    if let Ok(slot) = crate::metadata::mapped_slot(country, links) {
                        self.country_slots.entry(slot).or_default().push(index);
                    }
                }
            }
        }
        if changed(TableId::Focus) {
            self.focuses = frame
                .complete::<tables::Focus>()
                .ok()
                .map(|rows| rows.to_vec());
        }
        if changed(TableId::Focus) || changed(TableId::CountryLinks) {
            let next: BTreeMap<_, _> = self
                .focuses
                .iter()
                .flatten()
                .filter_map(|focus| {
                    let links = self.links.as_ref()?;
                    let index = links
                        .binary_search_by_key(&focus.country, |link| link.id)
                        .ok()?;
                    Some((links[index].canonical_slot, focus.clone()))
                })
                .collect();
            for slot in self.by_slot.keys().chain(next.keys()) {
                if self.by_slot.get(slot) != next.get(slot) {
                    dirty.extend(self.country_slots.get(slot).into_iter().flatten().copied());
                }
            }
            self.by_slot = next;
            self.completion_slots.clear();
            for (slot, row) in &self.by_slot {
                self.completion_slots
                    .entry(row.country)
                    .or_default()
                    .push(*slot);
            }
        }
        if changed(TableId::CompletedFocuses) {
            let next = frame.complete::<tables::CompletedFocuses>().ok();
            let owners: BTreeSet<_> = self
                .completed
                .iter()
                .flatten()
                .chain(next.into_iter().flatten())
                .map(|row| row.country)
                .collect();
            for owner in owners {
                let old = self
                    .completed
                    .as_deref()
                    .map_or(&[][..], |rows| completed_for_owner(rows, owner));
                let new = next.map_or(&[][..], |rows| completed_for_owner(rows, owner));
                if old != new {
                    for slot in self.completion_slots.get(&owner).into_iter().flatten() {
                        dirty.extend(self.country_slots.get(slot).into_iter().flatten().copied());
                    }
                }
            }
            self.completed = next.map(|rows| rows.to_vec());
        }
        if countries_changed || changed(TableId::CountryLinks) {
            dirty.extend(0..self.countries.as_ref().map_or(0, Vec::len));
        }
        dirty
    }
}

impl FocusAccumulator {
    /// Emit changed orders or progress (None), never an unchanged country-hour.
    pub(crate) fn observe_changes(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        cancelled: &AtomicBool,
        mut emit: impl FnMut(&str, Option<Observation>) -> io::Result<()>,
    ) -> io::Result<()> {
        check_cancel(cancelled)?;
        if self.branch.is_some_and(|branch| branch != frame.branch)
            || self.hour.is_some_and(|hour| hour >= frame.hour)
        {
            return Err(io::Error::other(
                "Focus preparation requires increasing hours in one branch",
            ));
        }
        self.removed.clear();
        let consecutive = self.hour.and_then(|hour| hour.checked_add(1)) == Some(frame.hour);
        let countries_changed = changes.rows_changed[TableId::Countries.index()]
            && !self.keys.as_ref().is_some_and(|keys| {
                frame.complete::<tables::Countries>().is_ok_and(|rows| {
                    #[cfg(test)]
                    let before = keys.row_visits();
                    let same = keys.same(rows);
                    #[cfg(test)]
                    {
                        self.work.row_visits += keys.row_visits() - before;
                    }
                    same
                })
            });
        if countries_changed {
            self.keys = frame
                .complete::<tables::Countries>()
                .ok()
                .map(CountryKeys::read);
        }
        let mut candidates = self.native.refresh(frame, changes, countries_changed);
        #[cfg(test)]
        {
            for table in TABLES[2..]
                .iter()
                .filter(|table| changes.rows_changed[table.index()])
            {
                self.work.row_visits += match &*frame.tables[table.index()] {
                    replay::frame::TableView::Complete(rows)
                    | replay::frame::TableView::Truncated { rows, .. } => rows.len(),
                    replay::frame::TableView::Unavailable { .. } => 0,
                };
            }
        }
        if consecutive
            && self.reusable_inputs
            && !countries_changed
            && TABLES[2..]
                .iter()
                .all(|table| !changes.rows_changed[table.index()])
        {
            self.hour = Some(frame.hour);
            return Ok(());
        }
        let had_inputs = self.reusable_inputs;
        self.reusable_inputs = false;
        let (Some(countries), Some(links), Some(_), Some(completed)) = (
            self.native.countries.as_deref(),
            self.native.links.as_deref(),
            self.native.focuses.as_deref(),
            self.native.completed.as_deref(),
        ) else {
            self.close_missing(&BTreeSet::new());
            self.branch = Some(frame.branch);
            self.hour = Some(frame.hour);
            return Ok(());
        };
        let unique = self.native.unique;
        let mut reusable = unique;
        let full = !had_inputs
            || !unique
            || !consecutive
            || countries_changed
            || changes.rows_changed[TableId::CountryLinks.index()];
        if full {
            candidates.extend(0..countries.len());
        }
        let by_slot = &self.native.by_slot;
        let mut active = BTreeSet::new();
        let mut touched = BTreeSet::new();
        for index in candidates {
            let country = &countries[index];
            if let Ok(tag) = std::str::from_utf8(&country.tag) {
                touched.insert(tag);
            }
            check_cancel(cancelled)?;
            #[cfg(test)]
            {
                self.work.row_visits += 1;
            }
            let Ok(slot) = crate::metadata::mapped_slot(country, links) else {
                continue;
            };
            let Some(focus) = by_slot.get(&slot) else {
                continue;
            };
            let Ok(tag) = std::str::from_utf8(&country.tag) else {
                continue;
            };
            if tag.is_empty() {
                continue;
            }
            let old = self
                .previous
                .get(tag)
                .filter(|old| (old.slot, old.country_id) == (slot, country.id));
            let last = if old.is_some_and(|old| old.progress.last == frame.hour) {
                Some(frame.hour)
            } else if self.active.contains(tag) {
                self.hour
            } else {
                old.map(|old| old.progress.last)
            };
            let same_token = |old: &Option<String>, raw: &Option<Vec<u8>>| {
                old.as_ref().map(|text| text.as_bytes())
                    == raw.as_deref().filter(|text| !text.is_empty())
            };
            let same_completed = had_inputs
                && unique
                && consecutive
                && !changes.rows_changed[TableId::CompletedFocuses.index()]
                && old.is_some_and(|old| {
                    old.completed_owner == focus.country
                        && usize::try_from(focus.completed_count).ok()
                            == Some(old.order.completed.len())
                });
            let rows = completed_for_owner(completed, focus.country);
            if !same_completed {
                #[cfg(test)]
                {
                    self.work.history_visits += rows.len();
                }
                if !complete_order(rows, focus.completed_count) {
                    continue;
                }
            }
            let unchanged_order = old.is_some_and(|old| {
                old.order.paused == (focus.paused != 0)
                    && same_token(&old.order.current, &focus.current)
                    && same_token(&old.order.continuous, &focus.continuous)
            }) && (same_completed
                || old.is_some_and(|old| {
                    old.order.completed.len() == rows.len()
                        && old
                            .order
                            .completed
                            .iter()
                            .zip(rows)
                            .all(|(entry, row)| entry.token.as_bytes() == row.token)
                }));
            active.insert(tag);
            if had_inputs && unique && consecutive && last == self.hour && unchanged_order {
                let old = self.previous.get_mut(tag).expect("observed focus country");
                old.completed_owner = focus.country;
                if old.progress.raw != focus.progress {
                    old.progress = ProgressBaseline {
                        first: frame.hour,
                        last: frame.hour,
                        raw: focus.progress,
                        previous: Some((
                            old.progress.raw,
                            last.expect("adjacent focus hour"),
                            old.order.current.clone(),
                        )),
                    };
                    emit(tag, None)?;
                }
                continue;
            }
            #[cfg(test)]
            {
                self.work.history_visits += rows.len();
            }
            let mut order = FocusOrder {
                completed: rows
                    .iter()
                    .map(|row| FocusEntry {
                        ordinal: row.ordinal,
                        token: String::from_utf8_lossy(&row.token).into_owned(),
                        completed_by_hour: None,
                    })
                    .collect(),
                current: focus
                    .current
                    .as_deref()
                    .filter(|token| !token.is_empty())
                    .map(|token| String::from_utf8_lossy(token).into_owned()),
                continuous: focus
                    .continuous
                    .as_deref()
                    .filter(|token| !token.is_empty())
                    .map(|token| String::from_utf8_lossy(token).into_owned()),
                paused: focus.paused != 0,
            };
            if let Some(old) = old {
                date_completions(&mut order, &old.order, frame.hour);
            }
            let progress = if let Some(old) = old
                && last.and_then(|hour| hour.checked_add(1)) == Some(frame.hour)
                && old.progress.raw == focus.progress
            {
                ProgressBaseline {
                    last: frame.hour,
                    ..old.progress.clone()
                }
            } else {
                ProgressBaseline {
                    first: frame.hour,
                    last: frame.hour,
                    raw: focus.progress,
                    previous: old.map(|old| {
                        (
                            old.progress.raw,
                            last.unwrap_or(old.progress.last),
                            old.order.current.clone(),
                        )
                    }),
                }
            };
            let raw_text_valid = focus
                .current
                .as_deref()
                .is_none_or(|token| std::str::from_utf8(token).is_ok())
                && focus
                    .continuous
                    .as_deref()
                    .is_none_or(|token| std::str::from_utf8(token).is_ok())
                && rows
                    .iter()
                    .all(|row| std::str::from_utf8(&row.token).is_ok());
            reusable &= raw_text_valid;
            let order_first_hour = old
                .filter(|old| {
                    raw_text_valid
                        && last.and_then(|hour| hour.checked_add(1)) == Some(frame.hour)
                        && old.order == order
                })
                .map_or(frame.hour, |old| old.order_first_hour);
            let observation = Observation {
                hour: frame.hour,
                order_first_hour,
                tag: tag.to_owned(),
                slot,
                country_id: country.id,
                order: order.clone(),
                progress_raw: focus.progress.to_string(),
                daily_gain_raw: daily_gain(&progress, &order).map(|value| value.to_string()),
            };
            emit(tag, Some(observation))?;
            self.previous.insert(
                tag.to_owned(),
                Previous {
                    slot,
                    country_id: country.id,
                    completed_owner: focus.country,
                    order,
                    order_first_hour,
                    progress,
                },
            );
        }
        let missing: Vec<_> = if full {
            self.active
                .iter()
                .filter(|tag| !active.contains(tag.as_str()))
                .cloned()
                .collect()
        } else {
            touched
                .difference(&active)
                .filter(|tag| self.active.contains(**tag))
                .map(|tag| (*tag).to_owned())
                .collect()
        };
        for tag in missing {
            if let Some(hour) = self.hour {
                self.previous
                    .get_mut(&tag)
                    .expect("active focus")
                    .progress
                    .last = hour;
            }
            self.active.remove(&tag);
            self.removed.push(tag);
        }
        for tag in active {
            self.active.insert(tag.to_owned());
        }
        self.reusable_inputs = reusable;
        self.branch = Some(frame.branch);
        self.hour = Some(frame.hour);
        Ok(())
    }

    fn close_missing(&mut self, active: &BTreeSet<&str>) {
        self.removed.extend(
            self.active
                .iter()
                .filter(|tag| !active.contains(tag.as_str()))
                .cloned(),
        );
        for tag in &self.removed {
            if let Some(hour) = self.hour {
                self.previous
                    .get_mut(tag)
                    .expect("active focus")
                    .progress
                    .last = hour;
            }
            self.active.remove(tag);
        }
        for tag in active {
            if !self.active.contains(*tag) {
                self.active.insert((*tag).to_owned());
            }
        }
    }

    pub(crate) fn observation(&self, tag: &str, hour: i32) -> Observation {
        let previous = &self.previous[tag];
        Observation {
            hour,
            order_first_hour: previous.order_first_hour,
            tag: tag.to_owned(),
            slot: previous.slot,
            country_id: previous.country_id,
            order: previous.order.clone(),
            progress_raw: previous.progress.raw.to_string(),
            daily_gain_raw: daily_gain(&previous.progress, &previous.order)
                .map(|value| value.to_string()),
        }
    }
}
fn daily_gain(progress: &ProgressBaseline, order: &FocusOrder) -> Option<i128> {
    progress.previous.as_ref().and_then(|(raw, last, current)| {
        if last.checked_add(1) != Some(progress.first)
            || order.current.is_none()
            || &order.current != current
        {
            return None;
        }
        let gain = i128::from(progress.raw) - i128::from(*raw);
        (gain > 0).then_some(gain)
    })
}

pub use artwork::FocusProjection as Presentation;

struct FocusBinding {
    source: crate::SourceKey,
    key: crate::catalogue::Binding,
    result: Result<(), crate::catalogue::ContentError>,
}

/// Current request operands: only clock/state-dependent tokens and their row
/// positions. Rebuilt from a changed order, never from retained history hourly.
#[derive(Default)]
struct HourlyFocus {
    modes: BTreeMap<Presentation, Vec<(String, Vec<usize>)>>,
}

/// Static results share the content binding; dynamic tokens retain hourly evaluation.
#[derive(Default)]
pub struct FocusEvaluator {
    binding: Option<FocusBinding>,
    context: Option<artwork::Context>,
    catalogue: Option<Arc<crate::catalogue::Catalogue>>,
    warnings: Vec<String>,
    static_text: BTreeSet<String>,
    presentations: BTreeMap<(Presentation, String, String), (bool, Option<artwork::ResolvedArt>)>,
    reusable: BTreeSet<String>,
    hourly: BTreeMap<String, HourlyFocus>,
    failed: BTreeSet<String>,
    failed_error: Option<String>,
    #[cfg(test)]
    test_changes: Option<crate::prepared::frame_changes::FrameChangeTracker>,
    #[cfg(test)]
    interpreted: usize,
    #[cfg(test)]
    interpreter_calls: usize,
    #[cfg(test)]
    legacy: bool,
    #[cfg(test)]
    scoped_input_builds: usize,
    #[cfg(test)]
    counts: FocusCounts,
}

#[cfg(test)]
#[derive(Default, Serialize)]
struct FocusCounts {
    observations: usize,
    unchanged: usize,
    changed_order: usize,
    progress_only: usize,
    progress_reused: usize,
    evaluated_dynamic: usize,
    evaluated_static: usize,
    refused_items: usize,
    failed_countries: usize,
    dynamic_tokens: BTreeMap<String, BTreeMap<String, usize>>,
    completed_rows_copied: usize,
}

impl FocusEvaluator {
    /// Bind fixed content before accepting frames. Failed bindings remain local
    /// to countries that actually ask for focus presentation, as before.
    pub(crate) fn configure(
        &mut self,
        source: &Source,
        content: &ContentOptions,
    ) -> io::Result<()> {
        if !self.binding.as_ref().is_some_and(|bound| {
            bound.source == *source.key() && crate::catalogue::binding_matches(&bound.key, content)
        }) {
            self.binding = None;
            self.context = None;
            self.catalogue = None;
            self.warnings.clear();
            self.static_text.clear();
            self.presentations.clear();
            self.reusable.clear();
            self.hourly.clear();
            self.failed.clear();
            self.failed_error = None;
            let result = (|| {
                let catalogue = crate::catalogue::catalogue(source, content)?;
                self.catalogue = Some(Arc::clone(&catalogue));
                let context = artwork::Context::load(catalogue, artwork::Kind::Focus)?;
                self.warnings = context.standing_warnings();
                self.context = Some(context);
                Ok(())
            })();
            if result
                .as_ref()
                .is_err_and(|error: &io::Error| error.kind() == io::ErrorKind::Interrupted)
            {
                return result;
            }
            self.binding = Some(FocusBinding {
                source: source.key().clone(),
                key: crate::catalogue::binding(content),
                result: result.map_err(crate::catalogue::ContentError::from),
            });
        }
        self.binding
            .as_ref()
            .expect("focus content bound")
            .result
            .as_ref()
            .map(|_| ())
            .map_err(crate::catalogue::ContentError::error)
    }

    pub(crate) fn reusable(&self, tag: &str) -> bool {
        self.reusable.contains(tag)
    }

    pub(crate) fn refuse(&mut self, tag: &str) {
        self.reusable.remove(tag);
    }
    #[allow(clippy::too_many_arguments)]
    pub fn present<T>(
        &mut self,
        source: &Source,
        content: &ContentOptions,
        frame: &DecodedFrame,
        tag: &str,
        tokens: &[String],
        mode: Presentation,
        cancelled: &AtomicBool,
        collect: impl FnOnce(
            &mut dyn Iterator<Item = artwork::ResolvedArt>,
            Vec<String>,
        ) -> io::Result<T>,
    ) -> io::Result<T> {
        check_cancel(cancelled)?;
        self.configure(source, content)?;
        let catalogue = Arc::clone(self.catalogue.as_ref().expect("focus catalogue loaded"));
        let inputs = artwork::FocusFrame::new(frame, &catalogue);
        let result = self.present_in(
            source, content, &inputs, tag, tokens, mode, cancelled, collect,
        );
        #[cfg(test)]
        {
            self.scoped_input_builds += inputs.text_builds.get();
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn present_in<T>(
        &mut self,
        source: &Source,
        content: &ContentOptions,
        inputs: &artwork::FocusFrame<'_>,
        tag: &str,
        tokens: &[String],
        mode: Presentation,
        cancelled: &AtomicBool,
        collect: impl FnOnce(
            &mut dyn Iterator<Item = artwork::ResolvedArt>,
            Vec<String>,
        ) -> io::Result<T>,
    ) -> io::Result<T> {
        check_cancel(cancelled)?;
        self.configure(source, content)?;
        #[cfg(test)]
        let legacy = self.legacy;
        let context = self.context.as_mut().expect("focus context loaded");
        // focus-artwork requests deduplicate and sort tokens before the interpreter.
        let tokens: BTreeSet<_> = tokens.iter().map(String::as_str).collect();
        let mut dynamic = false;
        let mut requests = Vec::new();
        let keys: Vec<_> = tokens
            .into_iter()
            .map(|token| {
                let token = token.to_owned();
                let scoped_icon = matches!(
                    mode,
                    Presentation::Display | Presentation::Icons | Presentation::Full
                ) && context.focus_country_icon_override(&token, tag);
                let key = (
                    mode,
                    token.clone(),
                    if scoped_icon {
                        tag.to_owned()
                    } else {
                        String::new()
                    },
                );
                let result = self.presentations.entry(key.clone()).or_insert_with(|| {
                    #[cfg(test)]
                    if legacy {
                        let tokens = std::slice::from_ref(&token);
                        let needs_frame = match mode {
                            Presentation::Display => {
                                context.requested_focus_display_needs_frame(tokens)
                            }
                            Presentation::Titles => {
                                context.requested_focus_title_needs_frame(tokens)
                            }
                            Presentation::Icons => context.requested_focus_icons_need_frame(tokens),
                            Presentation::Text => context.requested_focus_text_needs_frame(tokens),
                            Presentation::Full => context.requested_focus_needs_frame(tokens),
                        };
                        return (needs_frame, None);
                    }
                    (
                        context.requested_tree_focus_needs_frame(&token, tag, mode),
                        None,
                    )
                });
                let hourly = result.0;
                dynamic |= hourly;
                if result.1.is_none() || hourly {
                    #[cfg(test)]
                    {
                        if hourly {
                            self.counts.evaluated_dynamic += 1;
                            let tokens = std::slice::from_ref(&token);
                            let reasons =
                                self.counts.dynamic_tokens.entry(token.clone()).or_default();
                            if context.requested_focus_title_needs_frame(tokens) {
                                *reasons.entry("scripted_title".into()).or_default() += 1;
                            }
                            if matches!(mode, Presentation::Text | Presentation::Full)
                                && context.requested_focus_text_needs_frame(tokens)
                            {
                                *reasons.entry("scripted_text".into()).or_default() += 1;
                            }
                            if matches!(
                                mode,
                                Presentation::Display | Presentation::Icons | Presentation::Full
                            ) && if legacy {
                                context.requested_focus_icons_need_frame(tokens)
                            } else {
                                context.requested_tree_focus_needs_frame(
                                    &token,
                                    tag,
                                    Presentation::Icons,
                                )
                            } {
                                *reasons.entry("conditional_icon".into()).or_default() += 1;
                            }
                        } else {
                            self.counts.evaluated_static += 1;
                        }
                    }
                    requests.push(token);
                }
                key
            })
            .collect();
        if matches!(mode, Presentation::Display) {
            self.reusable.insert(tag.to_owned());
            self.hourly.remove(tag);
        }
        if dynamic {
            self.reusable.remove(tag);
            self.hourly.entry(tag.to_owned()).or_default();
        }
        #[cfg(test)]
        {
            self.interpreted += requests.len();
        }
        let requests: Vec<_> = requests.iter().map(|token| (token.as_str(), tag)).collect();
        let warnings = self.warnings.clone();
        let cache = &mut self.presentations;
        #[cfg(test)]
        let refused_items = &mut self.counts.refused_items;
        let collect = |items: &mut dyn Iterator<Item = artwork::ResolvedArt>| {
            let mut merged = keys.into_iter().map(|key| {
                let item = match &cache[&key] {
                    (false, Some(item)) => item.clone(),
                    (hourly, _) => {
                        let hourly = *hourly;
                        let item = items.next().expect("focus interpreter result");
                        if !hourly {
                            cache.insert(key, (false, Some(item.clone())));
                        }
                        item
                    }
                };
                #[cfg(test)]
                if item.1.is_some() {
                    *refused_items += 1;
                }
                item
            });
            collect(&mut merged, warnings)
        };
        if requests.is_empty() {
            return collect(&mut std::iter::empty());
        }
        #[cfg(test)]
        {
            self.interpreter_calls += 1;
        }
        context.focuses_in(&requests, inputs, true, mode, collect)
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn write_frame(
        &mut self,
        writer: &mut store::Writer,
        source: &Source,
        content: Option<&ContentOptions>,
        frame: &DecodedFrame,
        accumulator: &mut FocusAccumulator,
        index: &mut FocusIndex,
        preferred: Option<&str>,
        cancelled: &AtomicBool,
        changes: &FrameChanges,
    ) -> io::Result<(u64, Option<String>)> {
        check_cancel(cancelled)?;
        if let Some(content) = content
            && let Err(error) = self.configure(source, content)
            && error.kind() == io::ErrorKind::Interrupted
        {
            return Err(error);
        }
        let mut display_error = None;
        index.write_unavailable(writer, frame, cancelled)?;
        let mut observations = Vec::new();
        accumulator.observe_changes(frame, changes, cancelled, |tag, observation| {
            observations.push((tag.to_owned(), observation));
            Ok(())
        })?;
        for tag in &accumulator.removed {
            index.close(tag, frame.hour);
            self.hourly.remove(tag);
            self.reusable.remove(tag);
            self.failed.remove(tag);
        }
        let changed: BTreeSet<_> = observations.iter().map(|(tag, _)| tag.as_str()).collect();
        let hourly: Vec<_> = self
            .hourly
            .keys()
            .filter(|tag| !changed.contains(tag.as_str()) && accumulator.active.contains(*tag))
            .cloned()
            .collect();
        for tag in hourly {
            observations.push((tag, None));
        }
        let changed_failures: BTreeSet<_> = observations
            .iter()
            .filter_map(|(tag, _)| self.failed.contains(tag).then_some(tag.as_str()))
            .collect();
        let mut failures = (self.failed.len() - changed_failures.len()) as u64;
        if failures != 0 {
            display_error.clone_from(&self.failed_error);
        }
        #[cfg(test)]
        {
            self.counts.failed_countries += failures as usize;
        }
        observations.sort_by_key(|(tag, row)| {
            (
                preferred != Some(tag.as_str()),
                row.as_ref()
                    .map_or(accumulator.previous[tag].country_id, |row| row.country_id),
            )
        });
        let catalogue = self.catalogue.as_ref().map(Arc::clone);
        let inputs = std::sync::OnceLock::new();
        for (tag, observation) in observations {
            check_cancel(cancelled)?;
            #[cfg(test)]
            {
                self.counts.observations += 1;
                match &observation {
                    None => self.counts.unchanged += 1,
                    Some(row) if row.order_first_hour == frame.hour => {
                        self.counts.changed_order += 1
                    }
                    Some(_) => self.counts.progress_only += 1,
                }
            }
            if observation.is_none()
                && (self.reusable(&tag) || self.failed.contains(&tag))
                && index.write_progress(
                    frame.branch,
                    &tag,
                    frame.hour,
                    &accumulator.previous[&tag],
                    (!self.failed.contains(&tag)).then(|| self.warnings()),
                    cancelled,
                )?
            {
                if self.failed.contains(&tag) {
                    failures = failures
                        .checked_add(1)
                        .ok_or_else(|| crate::invalid("focus refusal count overflow"))?;
                    display_error.clone_from(&self.failed_error);
                    #[cfg(test)]
                    {
                        self.counts.failed_countries += 1;
                    }
                }
                #[cfg(test)]
                {
                    self.counts.progress_reused += 1;
                }
                continue;
            }
            #[cfg(test)]
            let can_refresh = !self.legacy;
            #[cfg(not(test))]
            let can_refresh = true;
            if can_refresh
                && observation.is_none()
                && !self.failed.contains(&tag)
                && let (Some(content), Some(catalogue)) = (content, catalogue.as_ref())
                && self
                    .hourly
                    .get(&tag)
                    .is_some_and(|layout| !layout.modes.is_empty())
            {
                let shared = inputs.get_or_init(|| artwork::FocusFrame::new(frame, catalogue));
                if self.refresh_hourly(
                    source,
                    content,
                    shared,
                    &tag,
                    &accumulator.previous[&tag],
                    index,
                    cancelled,
                )? {
                    continue;
                }
            }
            let observation =
                observation.unwrap_or_else(|| accumulator.observation(&tag, frame.hour));
            let mut tokens: Vec<_> = observation
                .order
                .completed
                .iter()
                .map(|entry| entry.token.clone())
                .collect();
            #[cfg(test)]
            {
                self.counts.completed_rows_copied += observation.order.completed.len();
            }
            tokens.extend(observation.order.current.iter().cloned());
            tokens.extend(observation.order.continuous.iter().cloned());
            let mut projections = Vec::new();
            let mut warnings = Vec::new();
            let mut failed = false;
            if let Some(content) = content
                && let Err(error) =
                    self.prepare_static_text(writer, source, content, &tokens, cancelled)
            {
                if error.kind() == io::ErrorKind::Interrupted {
                    return Err(error);
                }
                failed = true;
                display_error.get_or_insert_with(|| error.to_string());
                warnings.push(error.to_string());
            }
            for mode in [Presentation::Display, Presentation::Text] {
                let collect =
                    |items: &mut dyn Iterator<Item = crate::products::artwork::ResolvedArt>,
                     warnings| {
                        let values = items
                            .map(|value| {
                                check_cancel(cancelled)?;
                                Ok(value)
                            })
                            .collect::<io::Result<Vec<_>>>()?;
                        Ok((values, warnings))
                    };
                let presentation = match content {
                    Some(content) => {
                        let shared = catalogue.as_ref().map(|catalogue| {
                            inputs.get_or_init(|| artwork::FocusFrame::new(frame, catalogue))
                        });
                        #[cfg(test)]
                        let shared = shared.filter(|_| !self.legacy);
                        match shared {
                            Some(inputs) => self.present_in(
                                source,
                                content,
                                inputs,
                                &observation.tag,
                                &tokens,
                                mode,
                                cancelled,
                                collect,
                            ),
                            None => self.present(
                                source,
                                content,
                                frame,
                                &observation.tag,
                                &tokens,
                                mode,
                                cancelled,
                                collect,
                            ),
                        }
                    }
                    None => Err(io::Error::other(
                        "Choose the game and user folders for focus titles, icons and descriptions",
                    )),
                };
                match presentation {
                    Ok((items, reasons)) => {
                        projections.push(items);
                        warnings.extend(reasons);
                    }
                    Err(error) => {
                        if error.kind() == io::ErrorKind::Interrupted {
                            return Err(error);
                        }
                        failed = true;
                        display_error.get_or_insert_with(|| error.to_string());
                        warnings.push(error.to_string());
                        projections.push(Vec::new());
                    }
                }
            }
            if failed {
                #[cfg(test)]
                {
                    self.counts.failed_countries += 1;
                }
                self.refuse(&tag);
                failures = failures
                    .checked_add(1)
                    .ok_or_else(|| crate::invalid("focus refusal count overflow"))?;
                if content.is_none()
                    || self
                        .binding
                        .as_ref()
                        .is_some_and(|binding| binding.result.is_err())
                {
                    self.failed.insert(tag.clone());
                    self.failed_error.clone_from(&display_error);
                } else {
                    self.hourly.entry(tag.clone()).or_default();
                }
            } else {
                self.failed.remove(&tag);
            }
            warnings.sort();
            warnings.dedup();
            let mut projections = projections.into_iter();
            index.write_hour_with_details(
                writer,
                frame.branch,
                observation,
                &mut projections.next().unwrap().into_iter(),
                Some(&mut projections.next().unwrap().into_iter()),
                warnings,
                cancelled,
            )?;
            if let Some(layout) = self.hourly.get_mut(&tag) {
                let state = index.countries[&tag].last.as_ref().unwrap();
                layout.modes.clear();
                for (mode, rows) in [
                    (Presentation::Display, state.presentation.as_slice()),
                    (Presentation::Text, state.details.as_deref().unwrap_or(&[])),
                ] {
                    let mut tokens: BTreeMap<String, Vec<usize>> = BTreeMap::new();
                    for (position, row) in rows.iter().enumerate() {
                        let token = &row.artwork.token;
                        let dynamic = self
                            .presentations
                            .get(&(mode, token.clone(), tag.clone()))
                            .or_else(|| {
                                self.presentations
                                    .get(&(mode, token.clone(), String::new()))
                            })
                            .is_some_and(|result| result.0);
                        if dynamic {
                            tokens.entry(token.clone()).or_default().push(position);
                        }
                    }
                    if !tokens.is_empty() {
                        layout.modes.insert(mode, tokens.into_iter().collect());
                    }
                }
            }
        }
        #[cfg(test)]
        if let Some(inputs) = inputs.get() {
            self.scoped_input_builds += inputs.text_builds.get();
        }
        index.finish_frame(accumulator, frame.hour, cancelled)?;
        Ok((failures, display_error))
    }

    #[allow(clippy::too_many_arguments)]
    fn refresh_hourly(
        &mut self,
        source: &Source,
        content: &ContentOptions,
        inputs: &artwork::FocusFrame<'_>,
        tag: &str,
        previous: &Previous,
        index: &mut FocusIndex,
        cancelled: &AtomicBool,
    ) -> io::Result<bool> {
        let country = &index.countries[tag];
        let last = country.last.as_ref().unwrap();
        if (last.slot, last.country_id, last.order_first_hour)
            != (
                previous.slot,
                previous.country_id,
                previous.order_first_hour,
            )
        {
            return Ok(false);
        }
        let layout = self.hourly.remove(tag).unwrap();
        let mut update: Option<FocusState> = None;
        let result = (|| {
            let mut warnings = Vec::new();
            for mode in [Presentation::Display, Presentation::Text] {
                let Some(rows) = layout.modes.get(&mode) else {
                    continue;
                };
                let tokens: Vec<_> = rows.iter().map(|row| row.0.clone()).collect();
                let (evaluated, notes) = self.present_in(
                    source,
                    content,
                    inputs,
                    tag,
                    &tokens,
                    mode,
                    cancelled,
                    |items, warnings| {
                        let items = items
                            .map(|item| {
                                check_cancel(cancelled)?;
                                Ok(item)
                            })
                            .collect::<io::Result<Vec<_>>>()?;
                        Ok((items, warnings))
                    },
                )?;
                if evaluated.len() != rows.len() {
                    return Err(io::Error::other("Focus presentation omitted token"));
                }
                warnings.extend(notes);
                let old = if mode == Presentation::Display {
                    &last.presentation
                } else {
                    last.details.as_ref().unwrap()
                };
                for ((_, positions), (artwork, reason)) in rows.iter().zip(evaluated) {
                    let next = PresentedFocus { artwork, reason };
                    if old[positions[0]] != next {
                        if update.is_none() {
                            #[cfg(test)]
                            {
                                self.counts.completed_rows_copied += last.order.completed.len();
                            }
                            update = Some(last.clone());
                        }
                        let changed = update.as_mut().unwrap();
                        let target = if mode == Presentation::Display {
                            &mut changed.presentation
                        } else {
                            changed.details.as_mut().unwrap()
                        };
                        for &position in positions {
                            target[position] = next.clone();
                        }
                    }
                }
            }
            // Each projection has the same standing warnings; an earlier
            // transient write refusal needs the full path to retry its write.
            warnings.sort();
            warnings.dedup();
            Ok(warnings == last.warnings)
        })();
        self.hourly.insert(tag.to_owned(), layout);
        self.reusable.remove(tag);
        if !result? {
            return Ok(false);
        }
        let changed = update.is_some();
        if let Some(update) = update {
            let country = index.countries.get_mut(tag).unwrap();
            index
                .series
                .event(country.key, inputs.frame.hour, &update)?;
            country.last = Some(update);
        }
        index.progress(
            tag,
            inputs.frame.hour,
            i128::from(previous.progress.raw),
            daily_gain(&previous.progress, &previous.order).unwrap_or(0),
            changed,
        );
        Ok(true)
    }

    /// Store independent text for every observed token without consulting history.
    pub fn prepare_static_text(
        &mut self,
        writer: &mut store::Writer,
        source: &Source,
        content: &ContentOptions,
        tokens: &[String],
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        check_cancel(cancelled)?;
        self.configure(source, content)?;
        let context = self.context.as_mut().expect("focus context loaded");
        for token in tokens.iter().map(String::as_str).collect::<BTreeSet<_>>() {
            check_cancel(cancelled)?;
            if self.static_text.contains(token) {
                continue;
            }
            if writer.read_at(&static_text_section(token), 0)?.is_some() {
                self.static_text.insert(token.to_owned());
                continue;
            }
            let (mut items, _) = context.text_catalogue(BTreeSet::from([token]), cancelled)?;
            let item = items
                .pop()
                .ok_or_else(|| io::Error::other("Focus static text omitted token"))?;
            write_static_text(writer, &item, cancelled)?;
            self.static_text.insert(token.to_owned());
        }
        Ok(())
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "PreparedFocusArt")]
pub struct PresentedFocus {
    pub artwork: Artwork,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "PreparedFocusPages")]
pub struct Pages {
    #[serde(with = "decimal_u64")]
    #[ts(type = "string")]
    pub first: u64,
    #[serde(with = "decimal_u64")]
    #[ts(type = "string")]
    pub count: u64,
    #[serde(with = "decimal_u64")]
    #[ts(type = "string")]
    pub rows: u64,
}

mod decimal_u64 {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "PreparedFocusHour")]
pub struct Hour {
    #[ts(type = "number")]
    schema: StateSchema,
    pub slot: u32,
    pub country_id: u32,
    pub hour: i32,
    pub order_first_hour: i32,
    pub last_hour: i32,
    pub current: Option<String>,
    pub continuous: Option<String>,
    pub paused: bool,
    pub progress_raw: String,
    pub daily_gain_raw: Option<String>,
    pub completed: Pages,
    pub presentation: Pages,
    pub details: Option<Pages>,
    pub warnings: Vec<String>,
}

fn align_presentation(
    order: &FocusOrder,
    evaluated: Vec<PresentedFocus>,
) -> io::Result<Vec<PresentedFocus>> {
    // Interpretation retains sorted/deduplicated caller order; storage follows
    // recorded ordinals including repeats, with the active card at the tail.
    let by_token: BTreeMap<_, _> = evaluated
        .into_iter()
        .map(|item| (item.artwork.token.clone(), item))
        .collect();
    if by_token.is_empty() {
        return Ok(Vec::new());
    }
    order
        .completed
        .iter()
        .map(|entry| entry.token.as_str())
        .chain(order.current.as_deref().or(order.continuous.as_deref()))
        .map(|token| {
            by_token.get(token).cloned().ok_or_else(|| {
                io::Error::other(format!("Focus presentation omitted token {token}"))
            })
        })
        .collect()
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
struct FocusState {
    schema: StateSchema,
    slot: u32,
    country_id: u32,
    order_first_hour: i32,
    order: FocusOrder,
    presentation: Vec<PresentedFocus>,
    details: Option<Vec<PresentedFocus>>,
    warnings: Vec<String>,
}
#[derive(Default, Serialize, Deserialize)]
struct CountryState {
    key: u64,
    last: Option<FocusState>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "PreparedFocusTableRefusal")]
pub struct TableRefusal {
    pub table: String,
    pub reason: Option<String>,
    pub missing_rows: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "PreparedFocusUnavailableHour")]
pub struct UnavailableHour {
    #[ts(type = "number")]
    schema: StateSchema,
    pub first_hour: i32,
    pub last_hour: i32,
    pub reasons: Vec<TableRefusal>,
}
/// Current country state and shared series counters are the complete resume state.
#[derive(Default, Serialize, Deserialize)]
pub struct FocusIndex {
    completions: crate::products::national_focus::FocusCompletionProjection,
    completion_events: crate::prepared::series::Accumulator,
    completion_warnings: BTreeSet<String>,
    completion_presentation_refusal: Option<String>,
    #[serde(skip)]
    completion_art: Option<artwork::Context>,
    schema: StateSchema,
    branch: Option<u64>,
    countries: BTreeMap<String, CountryState>,
    hour: Option<i32>,
    #[serde(skip)]
    discontinuous: bool,
    series: Accumulator,
    reasons: Vec<TableRefusal>,
    #[serde(skip)]
    first: Option<i32>,
}
impl FocusIndex {
    pub(crate) fn observe_completions(
        &mut self,
        frame: &DecodedFrame,
        source: &Source,
        context: &crate::products::LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        check_cancel(cancelled)?;
        self.through(frame.branch, frame.hour)?;
        self.completion_events.through(frame.hour);
        if let Ok(content) = context.content()
            && let Err(reason) = artwork::ensure(
                &mut self.completion_art,
                source,
                content,
                artwork::Kind::Focus,
            )
        {
            self.completion_presentation_refusal
                .get_or_insert_with(|| reason.to_string());
        }
        let mut events = self.completions.observe(frame);
        events.sort_by(|a, b| (&a.tag, a.hour).cmp(&(&b.tag, b.hour)));
        let requests: Vec<_> = events
            .iter()
            .map(|event| (event.token.as_str(), event.tag.as_str()))
            .collect();
        let presented = if requests.is_empty() {
            Vec::new()
        } else {
            self.completion_art
                .as_mut()
                .map(|art| art.focuses_at(&requests, frame))
                .unwrap_or_default()
        };
        for (index, event) in events.into_iter().enumerate() {
            self.completion_events.event(
                0,
                event.hour,
                &Completion {
                    event,
                    presentation: presented
                        .get(index)
                        .map(|(artwork, reason)| PresentedFocus {
                            artwork: artwork.clone(),
                            reason: reason.clone(),
                        }),
                },
            )?;
        }
        if let Some(art) = &self.completion_art {
            self.completion_warnings.extend(art.standing_warnings());
        }
        Ok(())
    }
    fn through(&mut self, branch: u64, hour: i32) -> io::Result<()> {
        self.branch = Some(branch);
        self.first.get_or_insert(hour);
        if self.hour != Some(hour) {
            self.discontinuous = self.hour.and_then(|old| old.checked_add(1)) != Some(hour);
            self.hour = Some(hour);
        }
        self.series.advance(hour)
    }
    pub fn buffered_bytes(&self) -> usize {
        self.series.bytes() + self.completion_events.bytes()
    }
    fn progress(&mut self, tag: &str, hour: i32, raw: i128, gain: i128, changed: bool) {
        let country = self.countries.get(tag).expect("observed focus country");
        // Publication keeps the existing immutable interval boundary on resume.
        let break_before = changed || self.discontinuous || self.first == Some(hour);
        self.series
            .observe(country.key, hour, Some(raw), break_before);
        self.series
            .observe(country.key + 1, hour, Some(gain), break_before);
    }
    #[cfg(test)]
    pub(crate) fn extend_hour(
        &mut self,
        branch: u64,
        tag: &str,
        hour: i32,
        warnings: &[String],
    ) -> bool {
        let Some(country) = self.countries.get(tag) else {
            return false;
        };
        let Some(raw) = self
            .series
            .state(country.key)
            .and_then(|state| state.last)
            .filter(|run| run.last_hour.checked_add(1) == Some(hour))
        else {
            return false;
        };
        if country
            .last
            .as_ref()
            .is_none_or(|last| last.warnings != warnings)
        {
            return false;
        }
        let gain = self
            .series
            .state(country.key + 1)
            .and_then(|state| state.last)
            .expect("focus gain run")
            .value;
        if self.through(branch, hour).is_err() {
            return false;
        }
        self.progress(tag, hour, raw.value, gain, false);
        true
    }
    fn write_progress(
        &mut self,
        branch: u64,
        tag: &str,
        hour: i32,
        previous: &Previous,
        warnings: Option<&[String]>,
        cancelled: &AtomicBool,
    ) -> io::Result<bool> {
        check_cancel(cancelled)?;
        let Some(country) = self.countries.get(tag) else {
            return Ok(false);
        };
        if country.last.as_ref().is_none_or(|last| {
            (last.slot, last.country_id, last.order_first_hour)
                != (
                    previous.slot,
                    previous.country_id,
                    previous.order_first_hour,
                )
                || warnings.is_some_and(|warnings| last.warnings != warnings)
        }) {
            return Ok(false);
        }
        let raw = i128::from(previous.progress.raw);
        let gain = daily_gain(&previous.progress, &previous.order).unwrap_or(0);
        self.through(branch, hour)?;
        self.progress(tag, hour, raw, gain, false);
        Ok(true)
    }
    fn close(&mut self, tag: &str, hour: i32) {
        if let Some(country) = self.countries.get(tag) {
            self.series.observe(country.key, hour, None, false);
            self.series.observe(country.key + 1, hour, None, false);
        }
    }
    fn finish_frame(
        &mut self,
        accumulator: &FocusAccumulator,
        hour: i32,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        // Preserve focus's immutable publication boundaries. Only active keys
        // are visited once at a new publication, never at each carried hour.
        if self.first == Some(hour) {
            for tag in &accumulator.active {
                check_cancel(cancelled)?;
                if let Some(country) = self.countries.get(tag)
                    && self
                        .series
                        .state(country.key)
                        .and_then(|state| state.last)
                        .is_some_and(|run| run.last_hour != hour)
                {
                    let old = &accumulator.previous[tag];
                    self.progress(
                        tag,
                        hour,
                        i128::from(old.progress.raw),
                        daily_gain(&old.progress, &old.order).unwrap_or(0),
                        false,
                    );
                }
            }
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub fn write_hour_with_details(
        &mut self,
        _writer: &mut store::Writer,
        branch: u64,
        observation: Observation,
        presentation: &mut dyn Iterator<Item = artwork::ResolvedArt>,
        details: Option<&mut dyn Iterator<Item = artwork::ResolvedArt>>,
        warnings: Vec<String>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        check_cancel(cancelled)?;
        let (raw, gain) = observation.values()?;
        let collect = |items: &mut dyn Iterator<Item = artwork::ResolvedArt>| {
            let items = items
                .map(|(artwork, reason)| {
                    check_cancel(cancelled)?;
                    Ok(PresentedFocus { artwork, reason })
                })
                .collect::<io::Result<Vec<_>>>()?;
            align_presentation(&observation.order, items)
        };
        let state = FocusState {
            schema: StateSchema,
            slot: observation.slot,
            country_id: observation.country_id,
            order_first_hour: observation.order_first_hour,
            presentation: collect(presentation)?,
            details: details.map(collect).transpose()?,
            warnings,
            order: observation.order,
        };
        self.through(branch, observation.hour)?;
        let key = self.countries.len() as u64 * 2 + 1; // Key zero holds branch table refusals.
        let country = self
            .countries
            .entry(observation.tag.clone())
            .or_insert_with(|| CountryState {
                key,
                ..Default::default()
            });
        let changed = country.last.as_ref() != Some(&state);
        if changed {
            self.series.event(country.key, observation.hour, &state)?;
            country.last = Some(state);
        }
        self.progress(&observation.tag, observation.hour, raw, gain, changed);
        Ok(())
    }
    pub fn write_unavailable(
        &mut self,
        _writer: &mut store::Writer,
        frame: &DecodedFrame,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        use replay::frame::TableView;
        check_cancel(cancelled)?;
        self.through(frame.branch, frame.hour)?;
        let mut reasons = Vec::new();
        for (table, error) in [
            (
                TableId::Countries,
                frame.complete::<tables::Countries>().err(),
            ),
            (
                TableId::CountryLinks,
                frame.complete::<tables::CountryLinks>().err(),
            ),
            (TableId::Focus, frame.complete::<tables::Focus>().err()),
            (
                TableId::CompletedFocuses,
                frame.complete::<tables::CompletedFocuses>().err(),
            ),
        ] {
            let Some(error) = error else { continue };
            let (reason, missing_rows) = match &*frame.tables[table.index()] {
                TableView::Unavailable { reason } => (Some(reason.clone()), None),
                TableView::Truncated { missing, .. } => (None, Some(missing.to_string())),
                TableView::Complete(_) => (Some(error.to_string()), None),
            };
            reasons.push(TableRefusal {
                table: format!("{table:?}"),
                reason,
                missing_rows,
            });
        }
        let changed =
            self.series.state(0).is_none_or(|state| state.events == 0) || self.reasons != reasons;
        if changed {
            self.series.event(0, frame.hour, &reasons)?;
            self.reasons = reasons;
        }
        if changed || self.discontinuous || self.first == Some(frame.hour) {
            self.series.observe(0, frame.hour, Some(0), true);
        }
        Ok(())
    }
    pub fn flush(&mut self, writer: &mut store::Writer, cancelled: &AtomicBool) -> io::Result<()> {
        let Some(branch) = self.branch else {
            return Ok(());
        };
        self.completion_events.flush(
            writer,
            &format!("focus-completions-{branch}"),
            0,
            cancelled,
        )?;
        crate::prepared::series::write_catalogue(
            writer,
            "focus-completions",
            branch,
            &BTreeMap::new(),
            &(
                self.completions.observed,
                &self.completions.unavailable,
                &self.completion_warnings,
                &self.completion_presentation_refusal,
                crate::prepared::series::LateIndex::default(),
            ),
        )?;
        self.series
            .flush(writer, &format!("focus-{branch}"), 0, cancelled)?;
        self.first = None;
        let catalogue: BTreeMap<_, _> = self
            .countries
            .iter()
            .map(|(tag, country)| (tag, country.key))
            .collect();
        writer.write_at(
            &format!("focus-{branch}-catalogue"),
            0,
            serde_json::to_vec(&catalogue)?.as_slice(),
        )
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Completion {
    pub event: crate::products::national_focus::FocusCompletion,
    pub presentation: Option<PresentedFocus>,
}
fn static_text_section(token: &str) -> String {
    let mut name = String::from("focus-static-text-v1-");
    for byte in token.bytes() {
        use std::fmt::Write;
        write!(name, "{byte:02x}").expect("String formatting");
    }
    name
}
pub fn write_static_text(
    writer: &mut store::Writer,
    item: &Artwork,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    check_cancel(cancelled)?;
    writer.write_at(
        &static_text_section(&item.token),
        0,
        serde_json::to_vec(item)?.as_slice(),
    )
}
mod query;
pub use query::{
    details_page, order_page, presentation_page, query_hour, query_page, query_unavailable,
    static_text_page,
};

/// The runner unions these inputs with scripted-text inputs and Clock before decoding.
pub const TABLES: &[TableId] = crate::products::national_focus::FOCUS_COMPLETION_TABLES;
#[cfg(test)]
mod profile_tests;
#[cfg(test)]
mod tests;
