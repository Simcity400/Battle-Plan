//! The production topic observer feeds shared popularity runs and change events.
use super::*;
use dashboard_base::prepared::{
    coverage::Coverage,
    query::progressive,
    series::{self, Accumulator, Reader, Run},
};
use serde_json::{Value, json};

pub(crate) const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::PoliticalParties,
];

/// Native popularity, independent of naming catalogues and preceding frames.
/// An empty field requests every recorded popularity field of that tag.
#[cfg(test)]
pub(crate) fn sample(
    frame: &replay_frame::frame::DecodedFrame,
    requested: Option<&[(String, String)]>,
) -> io::Result<progressive::Samples> {
    sample_with_duplicates(frame, requested, &mut BTreeMap::new())
}

fn sample_with_duplicates(
    frame: &replay_frame::frame::DecodedFrame,
    requested: Option<&[(String, String)]>,
    duplicates: &mut BTreeMap<(String, String), (i32, i128)>,
) -> io::Result<progressive::Samples> {
    if requested.is_some_and(|keys| {
        keys.iter().any(|(_, field)| {
            !field.is_empty()
                && field
                    .strip_prefix("value:")
                    .is_none_or(|token| token.is_empty())
        })
    }) {
        return Err(dashboard_core::invalid(
            "Unknown party popularity sample field",
        ));
    }
    let mut sampled = BTreeMap::new();
    let wanted = requested.map(|keys| {
        keys.iter()
            .map(|(tag, field)| (tag.as_str(), field.as_str()))
            .collect::<BTreeSet<_>>()
    });
    if let (Ok(countries), Ok(links), Ok(parties)) = (
        frame.complete::<tables::Countries>(),
        frame.complete::<tables::CountryLinks>(),
        frame.complete::<tables::PoliticalParties>(),
    ) {
        let mut owned: BTreeMap<u32, Vec<_>> = BTreeMap::new();
        for row in parties {
            if let Some(slot) = link_slot(links, row.country) {
                owned.entry(slot).or_default().push(row);
            }
        }
        for (tag, found) in tagged_countries(countries, links) {
            let Ok(tag) = std::str::from_utf8(tag) else {
                continue;
            };
            let Some((country, slot)) = found.found() else {
                continue;
            };
            for row in owned.get(&slot).into_iter().flatten() {
                let Some((token, value)) = party_popularity(row) else {
                    continue;
                };
                let field = format!("value:{token}");
                if wanted.as_ref().is_none_or(|keys| {
                    keys.contains(&(tag, "")) || keys.contains(&(tag, field.as_str()))
                }) {
                    let key = (tag.to_owned(), field);
                    match sampled.entry(key.clone()) {
                        std::collections::btree_map::Entry::Vacant(entry) => {
                            entry.insert((Some((slot, country.id)), Some(value)));
                        }
                        std::collections::btree_map::Entry::Occupied(entry) => {
                            let first_value =
                                entry.get().1.expect("a native popularity was inserted");
                            duplicates
                                .entry(key)
                                .and_modify(|previous| {
                                    if frame.hour < previous.0 {
                                        *previous = (frame.hour, first_value);
                                    }
                                })
                                .or_insert((frame.hour, first_value));
                        }
                    }
                }
            }
        }
    }
    if let Some(requested) = requested {
        for key in requested {
            sampled.entry(key.clone()).or_insert((None, None));
        }
    }
    Ok(sampled)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum Event {
    Ruling {
        row: RulingRun,
        before: Option<RulingRun>,
    },
    Name {
        name: u32,
    },
    Leader {
        row: LeaderRun,
        before: Option<LeaderRun>,
    },
    Election,
}
/// One global census change. Its hour comes from the shared Events row; naming
/// operands retain their recorded success or refusal without resolving content.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RulingChange {
    pub tag: String,
    pub token: String,
    pub naming: Option<PartyNaming>,
    pub selected_tag: Option<Result<String, String>>,
}
#[derive(Default, Serialize, Deserialize)]
struct Metadata {
    first: Option<i32>,
    last: i32,
    countries: BTreeMap<String, ScannedParties>,
    types: BTreeMap<String, BTreeSet<String>>,
    duplicate: BTreeMap<String, i32>,
}
#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    scan: TopicScan,
    keys: BTreeMap<String, u64>,
    data: Accumulator,
    ruling_changes: Accumulator,
    metadata: Metadata,
    open: BTreeMap<String, BTreeSet<String>>,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    pub test_changes: Option<dashboard_base::prepared::frame_changes::FrameChangeTracker>,
}
fn key(keys: &mut BTreeMap<String, u64>, tag: &str, field: &str) -> u64 {
    let next = keys.len() as u64;
    *keys.entry(series::field_key(tag, field)).or_insert(next)
}
fn keep_last<T>(rows: &mut Vec<T>) {
    if rows.len() > 1 {
        rows.drain(..rows.len() - 1);
    }
}
/// Each interval names its metadata event. The low bit selects the marker key;
/// the remaining integer is its standard shared-series event ordinal.
fn interval(
    data: &mut Accumulator,
    keys: &mut BTreeMap<String, u64>,
    tag: &str,
    field: &str,
    run: Run,
    event: &Event,
    marker: bool,
) -> io::Result<()> {
    let changed = run.first_hour == run.last_hour;
    let id = key(keys, tag, field);
    let value = match (changed, data.state(id).and_then(|state| state.last)) {
        (false, Some(previous)) => previous.value,
        _ => {
            let event_key = key(
                keys,
                tag,
                &format!("{field}-{}", if marker { "marks" } else { "events" }),
            );
            let ordinal = data.state(event_key).map_or(0, |state| state.events);
            data.event(event_key, run.last_hour, event)?;
            i128::from(ordinal) * 2 + i128::from(marker)
        }
    };
    data.observe(id, run.last_hour, Some(value), changed && run.break_before);
    Ok(())
}
impl Prepared {
    fn append_ruling_changes(
        &mut self,
        hour: i32,
        mut changes: Vec<RulingChange>,
    ) -> io::Result<()> {
        // The feed seeks directly to a row's ordinal within a supplying hour.
        // Stable tag order keeps equal-tag changes in their observed order.
        changes.sort_by(|a, b| a.tag.cmp(&b.tag));
        for row in changes {
            self.ruling_changes.event(0, hour, &row)?;
        }
        Ok(())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn observe(&mut self, frame: &replay_frame::frame::DecodedFrame) -> io::Result<()> {
        let changes = self
            .test_changes
            .get_or_insert_with(dashboard_base::prepared::frame_changes::FrameChangeTracker::new)
            .observe(frame, replay_data::format::FormatVersion::CURRENT);
        self.observe_changes(frame, &changes)
    }
    pub fn observe_changes(
        &mut self,
        frame: &replay_frame::frame::DecodedFrame,
        changes: &dashboard_base::prepared::frame_changes::FrameChanges,
    ) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope_phase("parties:observe");
        self.metadata.first.get_or_insert(frame.hour);
        self.metadata.last = frame.hour;
        self.data.advance(frame.hour)?;
        self.scan.observe(frame, None, Some(changes));
        let unknown = self
            .scan
            .unobserved
            .last()
            .is_some_and(|span| span.last_hour == frame.hour);
        let unknown_key = key(&mut self.keys, "", "unavailable");
        if self
            .data
            .state(unknown_key)
            .and_then(|state| state.last)
            .is_none_or(|run| run.value != i128::from(unknown))
        {
            self.data
                .observe(unknown_key, frame.hour, Some(i128::from(unknown)), false);
        }
        self.scan.unobserved.clear();
        let mut ruling_changes = Vec::new();
        for tag in &self.scan.updated {
            let state = self
                .scan
                .countries
                .get_mut(tag)
                .expect("updated party holder");
            let result = &mut state.result;
            if state
                .open_parties
                .iter()
                .filter_map(|token| result.parties.get(token))
                .any(|party| {
                    party
                        .runs
                        .windows(2)
                        .any(|rows| rows[0].last_hour >= rows[1].first_hour)
                })
                || result
                    .ruling
                    .windows(2)
                    .any(|rows| rows[0].last_hour >= rows[1].first_hour)
            {
                self.metadata
                    .duplicate
                    .entry(tag.clone())
                    .or_insert(frame.hour);
            }
            self.metadata.types.entry(tag.clone()).or_default().extend(
                result
                    .leaders
                    .iter()
                    .map(|row| row.ideology.clone())
                    .filter(|token| !token.is_empty()),
            );
            if let Some(((slot, id), hour)) = state.last.filter(|last| last.1 == frame.hour) {
                let owner = key(&mut self.keys, tag, "owner");
                let identity = (i128::from(slot) << 32) | i128::from(id);
                let changed = self
                    .data
                    .state(owner)
                    .and_then(|state| state.last)
                    .is_none_or(|last| last.value != identity);
                self.data.observe(owner, hour, Some(identity), changed);
            }
            let mut open = BTreeSet::new();
            if state.last.is_some_and(|last| last.1 == frame.hour) {
                open.insert("owner".to_owned());
            }
            for token in &state.open_parties {
                let series = result.parties.get_mut(token).expect("open popularity");
                let field = format!("value:{token}");
                open.insert(field.clone());
                let id = key(&mut self.keys, tag, &field);
                let overlapping = self.scan.native.duplicates.contains(tag);
                if overlapping {
                    self.data.observe(id, frame.hour, None, false);
                }
                for row in series.runs.iter().filter(|row| row.last_hour == frame.hour) {
                    // Malformed duplicate rows retain their original values and overlap
                    // for the legacy validator; a scalar sample cannot encode duplicates.
                    let value = row
                        .value_raw
                        .parse::<i128>()
                        .map_err(|error| dashboard_core::invalid(error.to_string()))?;
                    if overlapping {
                        self.data.push_boundary(
                            id,
                            Run::new(
                                frame.hour,
                                frame.hour,
                                value,
                                row.first_hour == frame.hour && row.starts_segment,
                            ),
                            row.first_hour == frame.hour,
                        );
                    } else {
                        self.data.observe_boundary(
                            id,
                            frame.hour,
                            Some(value),
                            row.first_hour == frame.hour && row.starts_segment,
                            row.first_hour == frame.hour,
                        );
                    }
                }
                keep_last(&mut series.runs);
            }
            for (index, row) in result
                .ruling
                .iter()
                .enumerate()
                .filter(|(_, row)| row.last_hour == frame.hour)
            {
                open.insert("ruling".to_owned());
                let before = index
                    .checked_sub(1)
                    .and_then(|index| result.ruling.get(index))
                    .cloned();
                let marker = !row.starts_segment
                    && before.as_ref().is_some_and(|old| {
                        old.last_hour.checked_add(1) == Some(row.first_hour)
                            && old.token != row.token
                    });
                interval(
                    &mut self.data,
                    &mut self.keys,
                    tag,
                    "ruling",
                    Run::new(row.first_hour, row.last_hour, 0, row.starts_segment),
                    &Event::Ruling {
                        row: row.clone(),
                        before,
                    },
                    marker,
                )?;
                if marker {
                    let name = result
                        .party_name_runs
                        .iter()
                        .find(|name| {
                            name.first_hour <= row.first_hour && row.first_hour <= name.last_hour
                        })
                        .map(|name| name.name as usize);
                    ruling_changes.push(RulingChange {
                        tag: tag.clone(),
                        token: row.token.clone(),
                        naming: name.and_then(|name| result.namings.get(name)).cloned(),
                        selected_tag: name
                            .and_then(|name| result.selected_tags.get(name))
                            .cloned(),
                    });
                }
            }
            for row in result
                .party_name_runs
                .iter()
                .filter(|row| row.last_hour == frame.hour)
            {
                open.insert("name".to_owned());
                interval(
                    &mut self.data,
                    &mut self.keys,
                    tag,
                    "name",
                    Run::new(row.first_hour, row.last_hour, 0, false),
                    &Event::Name { name: row.name },
                    false,
                )?;
            }
            for (index, row) in result
                .leaders
                .iter()
                .enumerate()
                .filter(|(_, row)| row.last_hour == frame.hour)
            {
                open.insert("leader".to_owned());
                let before = index
                    .checked_sub(1)
                    .and_then(|index| result.leaders.get(index))
                    .cloned();
                let marker = !row.starts_segment
                    && before.as_ref().is_some_and(|old| {
                        old.last_hour.checked_add(1) == Some(row.first_hour)
                            && old.person.zip(row.person).is_some_and(|(was, now)| {
                                result.people[was as usize].character.id
                                    != result.people[now as usize].character.id
                            })
                    });
                interval(
                    &mut self.data,
                    &mut self.keys,
                    tag,
                    "leader",
                    Run::new(row.first_hour, row.last_hour, 0, row.starts_segment),
                    &Event::Leader {
                        row: row.clone(),
                        before,
                    },
                    marker,
                )?;
            }
            for hour in result.elections.drain(..) {
                self.data.event(
                    key(&mut self.keys, tag, "election-marks"),
                    hour,
                    &Event::Election,
                )?;
            }
            result.unobserved.clear();
            keep_last(&mut result.ruling);
            keep_last(&mut result.party_name_runs);
            keep_last(&mut result.leaders);
            let previous = self.open.entry(tag.clone()).or_default();
            for field in previous.difference(&open) {
                if let Some(id) = self.keys.get(&series::field_key(tag, field)) {
                    self.data.observe(*id, frame.hour, None, false);
                }
            }
            *previous = open;
        }
        for tag in &self.scan.removed {
            if let Some(fields) = self.open.remove(tag) {
                for field in fields {
                    if let Some(id) = self.keys.get(&series::field_key(tag, &field)) {
                        self.data.observe(*id, frame.hour, None, false);
                    }
                }
            }
        }
        self.append_ruling_changes(frame.hour, ruling_changes)
    }
    pub fn bytes(&self) -> usize {
        self.data.bytes() + self.ruling_changes.bytes()
    }
    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        // Undated catalogues are publication metadata, not per-frame rows.
        self.scan.seal();
        for (tag, state) in &self.scan.countries {
            let result = &state.result;
            let mut meta = ScannedParties {
                parties: result
                    .parties
                    .keys()
                    .map(|token| {
                        (
                            token.clone(),
                            PartySeries {
                                token: token.clone(),
                                runs: vec![],
                            },
                        )
                    })
                    .collect(),
                namings: result.namings.clone(),
                selected_tags: result.selected_tags.clone(),
                people: result.people.clone(),
                ..Default::default()
            };
            if !state.invalid_tokens.is_empty() {
                meta.warnings.push(format!("PoliticalParties ideology tokens are empty or not UTF-8; their popularity and names cannot be identified (count: {}).",state.invalid_tokens.len()));
            }
            self.metadata.countries.insert(tag.clone(), meta);
        }
        self.data.flush(
            writer,
            &format!("party-series-{branch}"),
            POPULARITY_DECIMALS,
            cancelled,
        )?;
        self.ruling_changes
            .flush(writer, &format!("party-ruling-{branch}"), 0, cancelled)?;
        series::write_catalogue(writer, "party-history", branch, &self.keys, &self.metadata)
    }
    pub fn saved_bytes(&self) -> io::Result<Vec<u8>> {
        dashboard_base::prepared::series::compress(serde_json::to_vec(self)?.as_slice(), 3)
    }
    pub fn read(reader: impl io::Read) -> io::Result<Self> {
        Ok(serde_json::from_reader(zstd::stream::Decoder::new(
            reader,
        )?)?)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Params {
    pub branch_id: String,
    tag: String,
    mode: String,
    #[serde(default = "values_default")]
    values: bool,
    #[serde(default)]
    fields: Option<Vec<String>>,
    within: Option<Vec<HourSpan>>,
    #[serde(default)]
    hours: Vec<i32>,
    from: Option<f64>,
    to: Option<f64>,
    width: Option<u32>,
    field: Option<String>,
    cursor: Option<String>,
    page_size: Option<u32>,
    navigation: Option<series::Navigation>,
}

fn within(spans: &Option<Vec<HourSpan>>, hour: i32) -> bool {
    spans.as_ref().is_none_or(|spans| {
        spans
            .iter()
            .any(|span| span.first_hour <= hour && span.last_hour >= hour)
    })
}

fn values_default() -> bool {
    true
}

fn field(keys: &BTreeMap<String, u64>, tag: &str, name: &str) -> Option<u64> {
    keys.get(&series::field_key(tag, name)).copied()
}

fn metadata_event(
    reader: &mut Reader<'_>,
    keys: &BTreeMap<String, u64>,
    tag: &str,
    name: &str,
    value: i128,
) -> io::Result<Event> {
    let value = u64::try_from(value)
        .map_err(|_| dashboard_core::invalid("Invalid party metadata ordinal"))?;
    let name = format!("{name}-{}", if value & 1 == 1 { "marks" } else { "events" });
    let key = field(keys, tag, &name)
        .ok_or_else(|| dashboard_core::invalid("Missing party metadata key"))?;
    let page = reader.events(&[key], i32::MIN, i32::MAX, value / 2, 1)?;
    serde_json::from_value(
        page.rows
            .into_iter()
            .next()
            .ok_or_else(|| dashboard_core::invalid("Missing party metadata event"))?
            .row,
    )
    .map_err(Into::into)
}

fn add_event(scanned: &mut ScannedParties, event: Event, run: Option<Run>, context: bool) {
    match event {
        Event::Ruling { mut row, before } => {
            if context && let Some(before) = before {
                scanned.ruling.push(before);
            }
            if let Some(run) = run {
                row.first_hour = run.first_hour;
                row.last_hour = run.last_hour;
            }
            scanned.ruling.push(row);
        }
        Event::Name { name } => {
            if let Some(run) = run {
                scanned.party_name_runs.push(PartyNameRun {
                    name,
                    first_hour: run.first_hour,
                    last_hour: run.last_hour,
                });
            }
        }
        Event::Leader { mut row, before } => {
            if context && let Some(before) = before {
                scanned.leaders.push(before);
            }
            if let Some(run) = run {
                row.first_hour = run.first_hour;
                row.last_hour = run.last_hour;
            }
            scanned.leaders.push(row);
        }
        Event::Election => {}
    }
}

fn unavailable(
    reader: &mut Reader<'_>,
    keys: &BTreeMap<String, u64>,
    tag: &str,
    from: i32,
    to: i32,
) -> io::Result<Vec<HourSpan>> {
    let mut spans = Vec::new();
    if let Some(key) = field(keys, "", "unavailable") {
        for run in reader
            .runs(key, from, to)?
            .into_iter()
            .filter(|run| run.value == 1)
        {
            spans.push(HourSpan {
                first_hour: run.first_hour,
                last_hour: run.last_hour,
            });
        }
    }
    if let Some(key) = field(keys, tag, "owner") {
        let mut owners = reader.runs(key, from, to)?;
        if let Some(hour) = reader.next_change(key, f64::from(from), false)?
            && let Some(run) = reader
                .value_at(key, hour)?
                .or(reader.value_at(key, hour.saturating_sub(1))?)
        {
            owners.insert(0, run);
        }
        if let Some(hour) = reader.next_change(key, f64::from(to), true)?
            && let Some(run) = reader.value_at(key, hour)?
        {
            owners.push(run);
        }
        for pair in owners.windows(2) {
            if pair[0].last_hour.saturating_add(1) < pair[1].first_hour {
                spans.push(HourSpan {
                    first_hour: pair[0].last_hour.saturating_add(1).max(from),
                    last_hour: pair[1].first_hour.saturating_sub(1).min(to),
                });
            }
        }
    }
    spans.sort_by_key(|span| span.first_hour);
    let mut result = Vec::new();
    for span in spans
        .into_iter()
        .filter(|span| span.first_hour <= span.last_hour)
    {
        mark_unobserved(&mut result, span.first_hour, span.last_hour);
    }
    Ok(result)
}

pub fn query(
    source: &Source,
    params: Params,
    context: &LoadContext,
    store: &store::Store,
    source_key: &store::SourceKey,
    accepted: Option<&Coverage>,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    super::CountryParams {
        branch_id: params.branch_id.clone(),
        tag: params.tag.clone(),
    }
    .validate()
    .map_err(dashboard_core::invalid)?;
    let branch = params
        .branch_id
        .parse()
        .map_err(|_| dashboard_core::invalid("Invalid party branch"))?;
    let bounds = dashboard_text::metadata::branch_bounds(
        source.replay_ref(),
        source
            .replay_ref()
            .branches
            .iter()
            .find(|start| start.id == branch)
            .ok_or_else(|| dashboard_core::invalid("Unknown party branch"))?,
    );
    let (keys, metadata) = if accepted.is_some() {
        series::read_catalogue::<Metadata>(store, source_key, "party-history", branch)?
            .unwrap_or_default()
    } else {
        Default::default()
    };
    let mut reader = if accepted.is_some() {
        Reader::new(
            store,
            source_key,
            format!("party-series-{branch}"),
            cancelled,
        )?
    } else {
        Reader::from_first_file(
            store,
            source_key,
            format!("party-series-{branch}"),
            0,
            None,
            cancelled,
        )
    };
    let partial = !accepted.is_some_and(|coverage| coverage.complete);
    let duplicates = std::cell::RefCell::new(BTreeMap::new());
    let native = |frame: &replay_frame::frame::DecodedFrame,
                  requested: Option<&[(String, String)]>| {
        sample_with_duplicates(frame, requested, &mut duplicates.borrow_mut())
    };
    let requested = [(params.tag.clone(), String::new())];
    let sampled = if partial && params.values {
        match params.mode.as_str() {
            "summary" => {
                let (from, to, _) = series::viewport(params.from, params.to, params.width)?;
                progressive::tail_projected(
                    source,
                    "party-history:popularity",
                    branch,
                    accepted,
                    TABLES,
                    &requested,
                    (from.floor() as i32, to.floor() as i32),
                    &(),
                    &native,
                    || std::mem::take(&mut *duplicates.borrow_mut()),
                    |held: &BTreeMap<(String, String), (i32, i128)>| {
                        let mut duplicates = duplicates.borrow_mut();
                        for (key, &(hour, value)) in held {
                            let row = duplicates.entry(key.clone()).or_insert((hour, value));
                            if hour < row.0 {
                                *row = (hour, value);
                            }
                        }
                    },
                    |held| {
                        let mut bytes = dashboard_base::prepared::prepass::HeldBytes::default();
                        bytes.value(held);
                        bytes.total() as usize
                    },
                    cancelled,
                )?
            }
            "state" => progressive::state_tail(
                source,
                branch,
                accepted,
                TABLES,
                &requested,
                &params.hours,
                &native,
                cancelled,
            )?,
            "next" => {
                let navigation = params
                    .navigation
                    .as_ref()
                    .ok_or_else(|| dashboard_core::invalid("Missing party navigation"))?;
                navigation.origin()?;
                progressive::tail(
                    source,
                    branch,
                    accepted,
                    TABLES,
                    &requested,
                    (
                        navigation.first_hour.floor() as i32,
                        navigation.last_hour.floor() as i32,
                    ),
                    &native,
                    cancelled,
                )?
            }
            _ => Default::default(),
        }
    } else {
        Default::default()
    };
    let mut scanned = metadata
        .countries
        .get(&params.tag)
        .cloned()
        .unwrap_or_default();
    for (tag, field) in sampled.0.keys().chain(duplicates.borrow().keys()) {
        if tag == &params.tag
            && let Some(token) = field.strip_prefix("value:")
        {
            scanned
                .parties
                .entry(token.to_owned())
                .or_insert_with(|| PartySeries {
                    token: token.to_owned(),
                    runs: Vec::new(),
                });
        }
    }
    let duplicate = metadata.duplicate.get(&params.tag).copied();
    let mut segments = Vec::new();
    let mut summaries = Vec::new();
    let mut marks = Vec::new();
    let mut cursor = None;
    let mut total = String::from("0");
    match params.mode.as_str() {
        "next" => {
            let navigation = params
                .navigation
                .ok_or_else(|| dashboard_core::invalid("Missing party navigation"))?;
            if partial {
                let mut prepared = BTreeMap::new();
                let mut fields: BTreeSet<_> = sampled
                    .0
                    .keys()
                    .filter(|(_, field)| !field.is_empty())
                    .cloned()
                    .collect();
                let origin = navigation.origin()?;
                let forward = matches!(
                    navigation.key,
                    series::MeasureKey::Home | series::MeasureKey::ArrowRight
                );
                for (name, &id) in &keys {
                    let tuple: (String, String) = serde_json::from_str(name)?;
                    if tuple.0 != params.tag
                        || tuple.1.ends_with("events")
                        || tuple.1.ends_with("marks")
                    {
                        continue;
                    }
                    let preceding = sampled
                        .0
                        .get(&tuple)
                        .and_then(|runs| runs.first())
                        .and_then(|run| run.first_hour.checked_sub(1))
                        .map(|hour| reader.value_at(id, hour))
                        .transpose()?
                        .flatten();
                    prepared.insert(
                        tuple.clone(),
                        (
                            reader.next_change(id, origin, forward)?.map(f64::from),
                            preceding,
                        ),
                    );
                    fields.insert(tuple);
                }
                let within = params
                    .within
                    .as_ref()
                    .map(|spans| BTreeMap::from([(params.tag.clone(), spans.clone())]))
                    .unwrap_or_default();
                return Ok(
                    json!({"hour": progressive::navigate(&prepared, &sampled.0, &fields.into_iter().collect::<Vec<_>>(), &within, &navigation)?}),
                );
            }
            let selected = keys
                .iter()
                .filter_map(|(name, &id)| {
                    let (tag, name): (String, String) = serde_json::from_str(name).ok()?;
                    (tag == params.tag && !name.ends_with("events") && !name.ends_with("marks"))
                        .then_some((id, None))
                })
                .collect();
            return Ok(
                json!({ "hour": series::navigate(&mut [(&mut reader, selected)], &navigation)? }),
            );
        }
        "summary" => {
            let (from, to, width) = series::viewport(params.from, params.to, params.width)?;
            series::summary_budget(from, to, width, scanned.parties.len())?;
            let window = [HourSpan {
                first_hour: from.floor() as i32,
                last_hour: to.floor() as i32,
            }];
            let memberships = params.within.as_deref().unwrap_or(&window);
            let mut spans: Vec<_> = sampled
                .1
                .iter()
                .flat_map(|span| {
                    memberships.iter().filter_map(move |within| {
                        let first = span
                            .first_hour
                            .max(within.first_hour)
                            .max(from.floor() as i32);
                        let last = span.last_hour.min(within.last_hour).min(to.floor() as i32);
                        (first <= last).then_some((first, last))
                    })
                })
                .collect();
            spans.sort();
            let mut approximate = Vec::new();
            for (first, last) in spans {
                mark_unobserved(&mut approximate, first, last);
            }
            for (token, party) in &mut scanned.parties {
                if !params.values {
                    continue;
                }
                let key = match field(&keys, &params.tag, &format!("value:{token}")) {
                    Some(key) => key,
                    None => keys
                        .values()
                        .max()
                        .copied()
                        .map_or(Some(0), |key| key.checked_add(1))
                        .ok_or_else(|| dashboard_core::invalid("Party key allocation overflow"))?,
                };
                let tail = sampled
                    .0
                    .get(&(params.tag.clone(), format!("value:{token}")))
                    .map_or(&[][..], Vec::as_slice);
                let mut summary = reader.summary_with_tail(
                    key,
                    bounds,
                    (from, to, width),
                    POPULARITY_DECIMALS,
                    tail,
                    params.within.as_deref(),
                )?;
                summary.approximate = approximate.clone();
                if let Some(runs) = &summary.runs {
                    for run in runs {
                        let raw = reader
                            .value_at(key, run.first_hour)?
                            .map(|row| row.value)
                            .or_else(|| {
                                tail.iter()
                                    .find(|row| {
                                        row.first_hour <= run.first_hour
                                            && row.last_hour >= run.first_hour
                                    })
                                    .map(|row| row.value)
                            })
                            .ok_or_else(|| {
                                dashboard_core::invalid("Missing exact party summary value")
                            })?;
                        party.runs.push(FixedRun {
                            first_hour: run.first_hour,
                            last_hour: run.last_hour,
                            value_raw: raw.to_string(),
                            starts_segment: run.break_before,
                        });
                    }
                }
                summaries.push(json!({"token": token, "summary": summary}));
            }
            let count = i64::from((to.floor() as i32).min(metadata.last))
                - i64::from((from.floor() as i32).max(metadata.first.unwrap_or(0)))
                + 1;
            let exact = count / i64::from(width) < 2;
            for name in ["ruling", "leader", "election"] {
                if params
                    .fields
                    .as_ref()
                    .is_some_and(|fields| !fields.iter().any(|field| field == name))
                {
                    continue;
                }
                if let Some(key) = field(&keys, &params.tag, &format!("{name}-marks")) {
                    let summary = if let Some(spans) = &params.within {
                        reader.event_clusters_within(key, from, to, width, spans)?
                    } else {
                        reader.event_clusters(key, from, to, width)?
                    };
                    let mut rows = Vec::new();
                    if exact {
                        let (first, last) = (from.ceil() as i32, to.floor() as i32);
                        if first <= last {
                            rows = reader
                                .events(
                                    &[key],
                                    first,
                                    last,
                                    0,
                                    width.saturating_mul(2).saturating_add(2),
                                )?
                                .rows;
                        }
                    } else {
                        for cluster in &summary.clusters {
                            if cluster.count == "1" {
                                rows.extend(
                                    reader
                                        .events(
                                            &[key],
                                            i32::MIN,
                                            i32::MAX,
                                            cluster.first_ordinal.parse().map_err(|_| {
                                                dashboard_core::invalid(
                                                    "Invalid party cluster ordinal",
                                                )
                                            })?,
                                            1,
                                        )?
                                        .rows,
                                );
                            }
                        }
                    }
                    for row in rows
                        .into_iter()
                        .filter(|row| within(&params.within, row.hour))
                    {
                        if name == "election" {
                            scanned.elections.push(row.hour);
                        } else {
                            add_event(&mut scanned, serde_json::from_value(row.row)?, None, true);
                        }
                    }
                    marks.push(json!({"field": name, "summary": summary, "exact": exact}));
                }
            }
        }
        "state" => {
            for &hour in &params.hours {
                if let Some(key) = field(&keys, &params.tag, "owner")
                    && let Some(run) = reader.value_at(key, hour)?
                {
                    segments.push(HourSpan {
                        first_hour: run.first_hour,
                        last_hour: run.last_hour,
                    });
                }
                for (token, party) in &mut scanned.parties {
                    let stored = field(&keys, &params.tag, &format!("value:{token}"))
                        .map(|key| reader.value_at(key, hour))
                        .transpose()?
                        .flatten();
                    if let Some(run) = stored.or_else(|| {
                        sampled
                            .0
                            .get(&(params.tag.clone(), format!("value:{token}")))
                            .and_then(|runs| {
                                runs.iter()
                                    .find(|run| run.first_hour <= hour && run.last_hour >= hour)
                            })
                            .copied()
                    }) {
                        party.runs.push(fixed_run(run));
                    }
                }
                if let Some((first, last, _, _)) = sampled
                    .2
                    .values()
                    .flatten()
                    .find(|(first, last, _, _)| *first <= hour && *last >= hour)
                {
                    segments.push(HourSpan {
                        first_hour: *first,
                        last_hour: *last,
                    });
                }
                for name in ["ruling", "name", "leader"] {
                    if let Some(key) = field(&keys, &params.tag, name)
                        && let Some(run) = reader.value_at(key, hour)?
                    {
                        add_event(
                            &mut scanned,
                            metadata_event(&mut reader, &keys, &params.tag, name, run.value)?,
                            Some(run),
                            false,
                        );
                    }
                }
                scanned.unobserved.extend(unavailable(
                    &mut reader,
                    &keys,
                    &params.tag,
                    hour,
                    hour,
                )?);
            }
        }
        "events" => {
            let name = params
                .field
                .as_deref()
                .filter(|name| ["ruling", "leader", "election", "all"].contains(name))
                .ok_or_else(|| dashboard_core::invalid("Invalid party event field"))?;
            let (from, to, offset, size) = series::event_window(
                params.from,
                params.to,
                params.cursor.as_deref(),
                Some(params.page_size.unwrap_or(128)),
            )?;
            let selected = ["ruling", "leader", "election"]
                .into_iter()
                .filter(|field| {
                    name == *field
                        || name == "all"
                            && params
                                .fields
                                .as_ref()
                                .is_none_or(|fields| fields.iter().any(|wanted| wanted == field))
                })
                .filter_map(|name| field(&keys, &params.tag, &format!("{name}-marks")))
                .collect::<Vec<_>>();
            let page = reader.events(&selected, from, to, offset, size)?;
            total = page.total.to_string();
            cursor = page.cursor.map(|ordinal| ordinal.to_string());
            for row in page
                .rows
                .into_iter()
                .filter(|row| within(&params.within, row.hour))
            {
                let event = serde_json::from_value(row.row)?;
                if matches!(event, Event::Election) {
                    scanned.elections.push(row.hour);
                } else {
                    add_event(&mut scanned, event, None, true);
                }
                scanned.unobserved.extend(unavailable(
                    &mut reader,
                    &keys,
                    &params.tag,
                    row.hour.saturating_sub(1),
                    row.hour,
                )?);
            }
        }
        "history" => {
            let from = metadata.first.unwrap_or(0);
            let to = metadata.last;
            for (token, party) in &mut scanned.parties {
                if let Some(key) = field(&keys, &params.tag, &format!("value:{token}")) {
                    party.runs = reader
                        .runs(key, from, to)?
                        .into_iter()
                        .map(fixed_run)
                        .collect();
                }
            }
            for name in ["ruling", "name", "leader"] {
                if let Some(key) = field(&keys, &params.tag, name) {
                    for run in reader.runs(key, from, to)? {
                        add_event(
                            &mut scanned,
                            metadata_event(&mut reader, &keys, &params.tag, name, run.value)?,
                            Some(run),
                            false,
                        );
                    }
                }
            }
            if let Some(key) = field(&keys, &params.tag, "election-marks") {
                let mut offset = 0;
                loop {
                    let page = reader.events(&[key], from, to, offset, 128)?;
                    scanned
                        .elections
                        .extend(page.rows.iter().map(|row| row.hour));
                    let Some(next) = page.cursor else { break };
                    offset = next;
                }
            }
            scanned.unobserved = unavailable(&mut reader, &keys, &params.tag, from, to)?;
        }
        _ => return Err(dashboard_core::invalid("Unknown party query mode")),
    }
    // The legacy validator refuses an overlapping vector even if its duplicate
    // lies outside today's window. Keep the original offending rows available
    // to that same validator, without an alternative refusal implementation.
    if let Some(hour) = duplicate
        && params.mode != "history"
    {
        for (token, party) in &mut scanned.parties {
            if let Some(key) = field(&keys, &params.tag, &format!("value:{token}")) {
                party
                    .runs
                    .extend(reader.runs(key, hour, hour)?.into_iter().map(fixed_run));
            }
        }
        if let Some(key) = field(&keys, &params.tag, "name") {
            for run in reader.runs(key, hour, hour)? {
                add_event(
                    &mut scanned,
                    metadata_event(&mut reader, &keys, &params.tag, "name", run.value)?,
                    Some(run),
                    false,
                );
            }
        }
    }
    for party in scanned.parties.values_mut() {
        party.runs.sort_by_key(|run| run.first_hour);
        if duplicate.is_none() {
            party.runs.dedup();
        }
    }
    for (tuple, (hour, value)) in duplicates.into_inner() {
        if tuple.0 == params.tag
            && let Some(token) = tuple.1.strip_prefix("value:")
        {
            let rows = &mut scanned
                .parties
                .get_mut(token)
                .expect("sampled token exists")
                .runs;
            rows.push(fixed_run(Run::new(hour, hour, value, true)));
            rows.push(fixed_run(Run::new(hour, hour, value, true)));
            rows.sort_by_key(|run| run.first_hour);
        }
    }
    scanned.ruling.sort_by_key(|run| run.first_hour);
    scanned.ruling.dedup_by(|next, previous| {
        if next.first_hour == previous.first_hour
            && next.token == previous.token
            && next.starts_segment == previous.starts_segment
        {
            previous.last_hour = previous.last_hour.max(next.last_hour);
            true
        } else {
            false
        }
    });
    scanned.leaders.sort_by_key(|run| run.first_hour);
    scanned.leaders.dedup_by(|next, previous| {
        if next.first_hour == previous.first_hour
            && next.ideology == previous.ideology
            && next.person == previous.person
            && next.starts_segment == previous.starts_segment
        {
            previous.last_hour = previous.last_hour.max(next.last_hour);
            true
        } else {
            false
        }
    });
    scanned.party_name_runs.sort_by_key(|run| run.first_hour);
    scanned.party_name_runs.dedup();
    scanned.unobserved.sort_by_key(|span| span.first_hour);
    for span in std::mem::take(&mut scanned.unobserved) {
        mark_unobserved(&mut scanned.unobserved, span.first_hour, span.last_hour);
    }
    let types = metadata.types.get(&params.tag);
    let reply = reply(
        source,
        &CountryParams {
            branch_id: params.branch_id,
            tag: params.tag,
        },
        context,
        &scanned,
        types,
        cancelled,
    )?;
    Ok(
        json!({"coverage": dashboard_base::prepared::prefix::extent(source, branch, accepted.cloned())?, "reply": reply, "segments": segments, "summaries": summaries, "marks": marks, "cursor": cursor, "total": total}),
    )
}

fn fixed_run(run: Run) -> FixedRun {
    FixedRun {
        first_hour: run.first_hour,
        last_hour: run.last_hour,
        value_raw: run.value.to_string(),
        starts_segment: run.break_before,
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::party_frame;
    use super::*;
    use dashboard_base::{
        prepared::series::tests::Fixture,
        test_fixtures::{CAMPAIGN, records, source_with_content},
    };
    use replay_data::{rows::CountryLinkRow, rows::PoliticalPartyRow, rows::TableRows};
    use replay_frame::frame::TableView;

    // Existing direct adapter fixtures publish a catalogue rather than the
    // pipeline's coverage page. Name that committed prefix explicitly.
    fn query(
        source: &Source,
        params: Params,
        context: &LoadContext,
        store: &store::Store,
        source_key: &store::SourceKey,
        cancelled: &AtomicBool,
    ) -> io::Result<Value> {
        let branch = params.branch_id.parse().unwrap();
        let (_, metadata) =
            series::read_catalogue::<Metadata>(store, source_key, "party-history", branch)?
                .unwrap();
        let bounds = dashboard_text::metadata::branch_bounds(
            source.replay_ref(),
            source
                .replay_ref()
                .branches
                .iter()
                .find(|start| start.id == branch)
                .unwrap(),
        );
        let accepted = Coverage {
            schema: 0,
            first_hour: metadata.first.unwrap(),
            last_hour: metadata.last,
            complete: metadata.last >= bounds.1,
        };
        super::query(
            source,
            params,
            context,
            store,
            source_key,
            Some(&accepted),
            cancelled,
        )
    }

    fn publish(state: &mut Prepared, fixture: &Fixture) {
        let flag = AtomicBool::new(false);
        let mut writer = fixture.writer();
        state.flush(&mut writer, 1, &flag).unwrap();
        writer
            .write_at(
                "party-history-1",
                0,
                state.saved_bytes().unwrap().as_slice(),
            )
            .unwrap();
        writer.commit().unwrap();
        *state = Prepared::read(
            fixture
                .store
                .read(&fixture.key, "party-history-1", 0)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            fixture
                .store
                .range_count(&fixture.key, "party-history-1")
                .unwrap(),
            Some(1)
        );
        assert!(
            state.scan.native.keys.is_none(),
            "transient native joins never survive resume"
        );
    }
    fn params(tag: &str, mode: &str, extra: Value) -> Params {
        let mut value = json!({"branchId":"1", "tag":tag, "mode":mode});
        value
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        serde_json::from_value(value).unwrap()
    }
    fn source(replay: Replay) -> Source {
        Source::from_replay(
            dashboard_base::source::SourceKey {
                source_id: "party-fixture".into(),
                revision: "1".into(),
            },
            "memory.battleplan".into(),
            Arc::new(replay),
            &AtomicBool::new(false),
        )
        .unwrap()
    }

    #[test]
    fn prepared_party_fractional_summary_keeps_values_without_an_integer_event_hour() {
        let frames = vec![
            party_frame(100, 4_000_001, 3_500_002, "fascism", 10),
            party_frame(101, 4_000_001, 3_500_002, "democratic", 10),
        ];
        let source =
            source(Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap());
        let fixture = Fixture::new();
        let mut state = Prepared::default();
        for frame in &frames {
            state.observe(frame).unwrap();
            publish(&mut state, &fixture);
        }
        let summary = |from, to| {
            query(
                &source,
                params("GER", "summary", json!({"from":from,"to":to,"width":10})),
                &LoadContext { content: None },
                &fixture.store,
                &fixture.key,
                &AtomicBool::new(false),
            )
            .unwrap()
        };
        for (from, to) in [(100.25, 100.75), (100.5, 100.5)] {
            let data = summary(from, to);
            let parties = data["reply"]["parties"].as_array().unwrap();
            let fascism = parties
                .iter()
                .find(|party| party["token"] == "fascism")
                .unwrap();
            assert_eq!(fascism["runs"][0]["valueRaw"], "4000001");
            assert_eq!(data["reply"]["ruling"], json!([]));
            assert_eq!(
                data["marks"].as_array().unwrap().len(),
                1,
                "a persisted ruling-marker key exercises the event path"
            );
            assert_eq!(data["marks"][0]["summary"]["clusters"], json!([]));
            assert_eq!(data["marks"][0]["exact"], true);
        }
        for (from, to) in [(100.25, 101.0), (101.0, 101.0)] {
            let data = summary(from, to);
            let ruling = data["reply"]["ruling"].as_array().unwrap();
            assert_eq!(ruling.last().unwrap()["firstHour"], 101);
            assert_eq!(ruling.last().unwrap()["token"], "democratic");
            assert_eq!(data["marks"][0]["summary"]["clusters"][0]["count"], "1");
        }
    }

    #[test]
    fn prepared_party_unavailable_finds_a_gap_after_a_single_hour_holder_across_commits() {
        let fixture = Fixture::new();
        let flag = AtomicBool::new(false);
        let mut data = Accumulator::default();
        for (hour, identity) in [(104, 1), (106, 2)] {
            data.observe(0, hour, Some(identity), true);
            let mut writer = fixture.writer();
            data.flush(&mut writer, "party-series-1", POPULARITY_DECIMALS, &flag)
                .unwrap();
            writer.commit().unwrap();
        }
        let keys = BTreeMap::from([(series::field_key("GER", "owner"), 0)]);
        let mut reader =
            Reader::new(&fixture.store, &fixture.key, "party-series-1".into(), &flag).unwrap();
        assert_eq!(
            unavailable(&mut reader, &keys, "GER", 105, 105).unwrap(),
            vec![HourSpan {
                first_hour: 105,
                last_hour: 105
            }]
        );
        for hour in [104, 106] {
            assert!(
                unavailable(&mut reader, &keys, "GER", hour, hour)
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[test]
    fn restored_party_history_closes_a_holder_that_vanishes_at_the_first_hour_after_a_commit() {
        // FRA has a ruling party until hour 104, the first hour after a commit.
        let frames: Vec<_> = (100..108)
            .map(|hour| {
                let mut frame = party_frame(hour, 40_00000, 35_00000, "fascism", 10);
                if hour >= 104
                    && let TableView::Complete(TableRows::Countries(rows)) =
                        &mut *frame.tables[TableId::Countries.index()]
                {
                    rows.retain(|row| row.id != 44);
                }
                frame
            })
            .collect();
        let flag = AtomicBool::new(false);
        dashboard_base::prepared::observer_tests::assert_restore_continues(
            replay_data::format::FormatVersion::CURRENT,
            &[&frames[..4], &frames[4..]],
            Prepared::default,
            |state, frame, changes, _| state.observe_changes(frame, changes).unwrap(),
            |state, writer, _| {
                state.flush(writer, 1, &flag).unwrap();
                writer
                    .write_at(
                        "party-history-1",
                        0,
                        state.saved_bytes().unwrap().as_slice(),
                    )
                    .unwrap();
            },
            |fixture| {
                Prepared::read(
                    fixture
                        .store
                        .read(&fixture.key, "party-history-1", 0)
                        .unwrap()
                        .unwrap(),
                )
                .unwrap()
            },
        );
    }

    #[test]
    fn prepared_party_replies_match_oracle_after_two_commits_resume_and_append() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let root =
            std::env::temp_dir().join(format!("prepared-party-content-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("common/ideologies")).unwrap();
        std::fs::create_dir_all(root.join("localisation/english")).unwrap();
        std::fs::write(root.join("common/ideologies/mod.txt"), "ideologies = { democratic = { types = { mod_dem = {} } color = { 20 40 60 } } fascism = { types = { mod_fas = {} } color = { 60 40 20 } } }").unwrap();
        std::fs::write(root.join("localisation/english/mod_l_english.yml"), "l_english:\n democratic:0 \"Mod democracy\"\n fascism:0 \"Mod faction\"\n custom_party:0 \"Literal [ROOT.GetName]\"\n refused_party:0 \"Control¤byte\"\n").unwrap();
        let make = |hour, ruling: &str, leader, name: &str| {
            let mut frame = party_frame(
                hour,
                4000001,
                3500002,
                ruling,
                if hour < 104 { 10 } else { 104 },
            );
            *frame.tables[TableId::CountryAppearance.index()] =
                TableView::Complete(TableRows::CountryAppearance(vec![
                    replay_data::rows::CountryAppearanceRow {
                        country: 9001,
                        leader_character: Some(leader),
                        leader_ideology: b"mod_dem".to_vec(),
                        original_tag_reference: replay_data::rows::Recorded::Value(Some(55)),
                        original_tag: replay_data::rows::Recorded::Value(Some(Vec::new())),
                        ..Default::default()
                    },
                ]));
            *frame.tables[TableId::Characters.index()] =
                TableView::Complete(TableRows::Characters(vec![
                    replay_data::rows::CharacterRow {
                        id: leader,
                        token: name.as_bytes().to_vec(),
                        ..Default::default()
                    },
                ]));
            if let TableView::Complete(TableRows::PoliticalParties(rows)) =
                &mut *frame.tables[TableId::PoliticalParties.index()]
            {
                let row = rows
                    .iter_mut()
                    .find(|row| row.country == 9001 && row.is_ruling != 0)
                    .unwrap();
                if hour == 102 {
                    row.name_default = 0;
                    row.name = b"refused_party".to_vec();
                }
                if hour >= 104 {
                    row.name_default = 0;
                    row.name = b"custom_party".to_vec();
                }
            }
            if hour >= 106 {
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
                {
                    rows.iter_mut().find(|row| row.id == 9001).unwrap().id = 9100;
                    rows.sort_by_key(|row| row.id);
                }
                if let TableView::Complete(TableRows::CountryLinks(rows)) =
                    &mut *frame.tables[TableId::CountryLinks.index()]
                {
                    rows.push(CountryLinkRow {
                        id: 9100,
                        canonical_slot: 7,
                    });
                    rows.sort_by_key(|row| row.id);
                }
                if let TableView::Complete(TableRows::CountryAppearance(rows)) =
                    &mut *frame.tables[TableId::CountryAppearance.index()]
                {
                    rows[0].country = 9100;
                }
            }
            if hour == 107 {
                *frame.tables[TableId::CharacterPortraits.index()] =
                    TableView::Complete(TableRows::CharacterPortraits(vec![
                        replay_data::rows::CharacterPortraitRow {
                            character: leader,
                            portraits: vec![replay_data::rows::CharacterPortrait {
                                category: replay_data::rows::PORTRAIT_CIVILIAN,
                                size: replay_data::rows::PORTRAIT_LARGE,
                                is_set: 1,
                                kind: 0,
                                identity: b"mod_portrait".to_vec(),
                            }],
                        },
                    ]));
            }
            frame
        };
        let mut absent = make(103, "fascism", 7, "Renamed leader");
        *absent.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
            reason: "synthetic links refusal".into(),
        };
        let frames = [
            make(100, "fascism", 7, "First leader"),
            make(101, "fascism", 7, "First leader"),
            make(102, "fascism", 7, "Renamed leader"),
            absent,
            make(104, "democratic", 8, "Next leader"),
            make(106, "democratic", 8, "Next leader"),
            make(107, "fascism", 8, "Next leader"),
            make(108, "fascism", 9, "Final leader"),
        ];
        let (first_source, context) = source_with_content(frames[..2].to_vec(), &root);
        let mut state = Prepared::default();
        let mut fixture = Fixture::new();
        let flag = AtomicBool::new(false);
        let compare = |fixture: &Fixture, source: &Source| {
            for tag in ["GER", "FRA", "CTL", "missing / tag"] {
                let old = PartyHistory::load(
                    source,
                    &CountryParams {
                        branch_id: "1".into(),
                        tag: tag.into(),
                    },
                    &context,
                    &flag,
                )
                .unwrap();
                let prepared = query(
                    source,
                    params(tag, "history", json!({})),
                    &context,
                    &fixture.store,
                    &fixture.key,
                    &flag,
                )
                .unwrap();
                assert_eq!(
                    prepared["reply"],
                    serde_json::to_value(old).unwrap(),
                    "{tag}"
                );
            }
        };
        for frame in &frames[..2] {
            state.observe(frame).unwrap();
        }
        publish(&mut state, &fixture);
        compare(&fixture, &first_source);
        for frame in &frames[2..5] {
            state.observe(frame).unwrap();
        }
        publish(&mut state, &fixture);
        let next = first_source
            .replay()
            .append(&[dashboard_base::test_fixtures::continuation_records(
                first_source.replay_ref(),
                1,
                frames[2..5].to_vec(),
            )])
            .unwrap();
        let second_source = source(next);
        compare(&fixture, &second_source);
        fixture.append_writer().commit().unwrap();
        for frame in &frames[5..] {
            state.observe(frame).unwrap();
        }
        publish(&mut state, &fixture);
        let next = second_source
            .replay()
            .append(&[dashboard_base::test_fixtures::continuation_records(
                second_source.replay_ref(),
                1,
                frames[5..].to_vec(),
            )])
            .unwrap();
        let final_source = source(next);
        compare(&fixture, &final_source);
        let exact = query(
            &final_source,
            params("GER", "state", json!({"hours":[102,103,104,105,106]})),
            &context,
            &fixture.store,
            &fixture.key,
            &flag,
        )
        .unwrap();
        assert_eq!(
            exact["reply"]["unobserved"],
            json!([{"firstHour":103,"lastHour":103},{"firstHour":105,"lastHour":105}])
        );
        let page = query(
            &final_source,
            params(
                "GER",
                "events",
                json!({"field":"election","from":100,"to":106,"pageSize":1}),
            ),
            &context,
            &fixture.store,
            &fixture.key,
            &flag,
        )
        .unwrap();
        assert_eq!(page["reply"]["elections"], json!([104]));
        assert_eq!(
            fixture
                .store
                .range_count(&fixture.key, "party-series-1")
                .unwrap(),
            Some(3)
        );
        let summary = query(
            &final_source,
            params(
                "GER",
                "summary",
                json!({"from":100.25,"to":100.75,"width":200}),
            ),
            &context,
            &fixture.store,
            &fixture.key,
            &flag,
        )
        .unwrap();
        assert_eq!(summary["reply"]["parties"][0]["runs"][0]["firstHour"], 100);
        assert_eq!(summary["reply"]["parties"][0]["runs"][0]["lastHour"], 102);
        for (field, hour) in [("ruling", 107), ("leader", 108)] {
            let page = query(
                &final_source,
                params(
                    "GER",
                    "events",
                    json!({"field":field,"from":100,"to":108,"pageSize":1}),
                ),
                &context,
                &fixture.store,
                &fixture.key,
                &flag,
            )
            .unwrap();
            assert_eq!(page["total"], "1");
            let rows = page["reply"][if field == "ruling" {
                "ruling"
            } else {
                "leaders"
            }]
            .as_array()
            .unwrap();
            assert_eq!(rows.last().unwrap()["firstHour"], hour);
        }
        let clipped = query(&final_source, params("GER","summary",json!({"from":100,"to":108,"width":1,"values":false,"fields":["ruling"],"within":[{"firstHour":100,"lastHour":106}]})), &context, &fixture.store, &fixture.key, &flag).unwrap();
        assert_eq!(clipped["marks"][0]["summary"]["clusters"], json!([]));
        std::fs::remove_dir_all(root).unwrap();
    }

    fn party_census(count: u32) -> replay_frame::frame::DecodedFrame {
        use replay_data::rows::CountryRow;
        let mut frame = dashboard_base::test_fixtures::ownership_frame(1, 0, 44);
        let countries: Vec<_> = (1..=count)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                tag: format!("country / {id}").into_bytes(),
                ..Default::default()
            })
            .collect();
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(
                countries
                    .iter()
                    .map(|row| CountryLinkRow {
                        id: row.id,
                        canonical_slot: row.canonical_slot,
                    })
                    .collect(),
            ));
        *frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(countries));
        *frame.tables[TableId::PoliticalParties.index()] =
            TableView::Complete(TableRows::PoliticalParties(
                (1..=count)
                    .map(|country| PoliticalPartyRow {
                        country,
                        id: 1,
                        tag: country,
                        token: b"mod_party".to_vec(),
                        name_default: 1,
                        is_ruling: 1,
                        popularity: 4000001,
                        ..Default::default()
                    })
                    .collect(),
            ));
        frame
    }

    fn assert_global_ruling_union(state: &Prepared, fixture: &Fixture, count: u64) {
        let flag = AtomicBool::new(false);
        let mut reader =
            Reader::new(&fixture.store, &fixture.key, "party-series-1".into(), &flag).unwrap();
        let mut expected = Vec::new();
        let mut requests = 0;
        for (tag, metadata) in &state.metadata.countries {
            let Some(id) = field(&state.keys, tag, "ruling-marks") else {
                continue;
            };
            let page = reader.events(&[id], i32::MIN, i32::MAX, 0, 128).unwrap();
            requests += 1;
            assert!(
                page.cursor.is_none(),
                "this fixture has at most three changes per tag"
            );
            for event in page.rows {
                let Event::Ruling { row, .. } = serde_json::from_value(event.row).unwrap() else {
                    panic!("a ruling marker must contain a ruling event");
                };
                assert!(!row.starts_segment);
                assert_eq!(row.first_hour, event.hour);
                let name = field(&state.keys, tag, "name")
                    .and_then(|id| reader.value_at(id, event.hour).unwrap())
                    .map(|run| {
                        let Event::Name { name } =
                            metadata_event(&mut reader, &state.keys, tag, "name", run.value)
                                .unwrap()
                        else {
                            panic!("a naming run must reference a name event");
                        };
                        name as usize
                    });
                expected.push((
                    event.hour,
                    RulingChange {
                        tag: tag.clone(),
                        token: row.token,
                        naming: name.and_then(|name| metadata.namings.get(name)).cloned(),
                        selected_tag: name
                            .and_then(|name| metadata.selected_tags.get(name))
                            .cloned(),
                    },
                ));
            }
        }
        assert!(
            requests <= state.metadata.countries.len(),
            "one bounded per-tag oracle page"
        );
        expected.sort_by(|a, b| (a.0, &a.1.tag).cmp(&(b.0, &b.1.tag)));
        assert_eq!(
            expected.len() as u64,
            count,
            "identity starts, gaps and renames are not ruling changes"
        );
        let mut global =
            Reader::new(&fixture.store, &fixture.key, "party-ruling-1".into(), &flag).unwrap();
        let mut actual = Vec::new();
        let mut cursor = 0;
        let mut pages = 0;
        loop {
            pages += 1;
            assert!(
                pages <= count.div_ceil(128).max(1),
                "global requests are bounded by the reached rows"
            );
            let page = global
                .events(&[0], i32::MIN, i32::MAX, cursor, 128)
                .unwrap();
            assert_eq!(page.total, count);
            actual.extend(page.rows.into_iter().map(|event| {
                (
                    event.hour,
                    serde_json::from_value::<RulingChange>(event.row).unwrap(),
                )
            }));
            let Some(next) = page.cursor else { break };
            assert!(next > cursor, "a global page must advance");
            cursor = next;
        }
        assert_eq!(
            actual, expected,
            "global rows preserve the per-tag changes and recorded naming operands"
        );
        let foreign = actual
            .iter()
            .find(|(_, row)| row.tag == "country / 1")
            .unwrap();
        assert_eq!(
            foreign.1.naming.as_ref().unwrap().key.as_deref(),
            Some("stored/mod:1:initial")
        );
        assert_eq!(
            foreign.1.naming.as_ref().unwrap().tag,
            Some(format!("country / {}", state.metadata.countries.len())),
            "global naming retains the party's foreign tag"
        );
        assert!(
            matches!(foreign.1.selected_tag.as_ref(), Some(Err(_))),
            "unread naming operands retain their refusal"
        );
    }

    fn global_ruling_stream_commits_resume_and_append(count: u32) {
        let mut frame = party_census(count);
        let mut state = Prepared::default();
        let mut fixture = Fixture::new();
        state.observe(&frame).unwrap();
        assert_eq!(
            state.ruling_changes.bytes(),
            0,
            "initial sightings emit no global change"
        );
        for hour in 1..=9 {
            frame.hour = hour;
            if hour == 4 {
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
                {
                    rows[0].canonical_slot = count + 1;
                }
                if let TableView::Complete(TableRows::CountryLinks(rows)) =
                    &mut *frame.tables[TableId::CountryLinks.index()]
                {
                    rows[0].canonical_slot = count + 1;
                }
            }
            if hour == 5 {
                *frame.tables[TableId::PoliticalParties.index()] = TableView::Unavailable {
                    reason: "synthetic party gap".into(),
                };
            } else if [1, 2, 4, 6, 8].contains(&hour) {
                let mut parties = party_census(count)
                    .complete::<tables::PoliticalParties>()
                    .unwrap()
                    .to_vec();
                for party in &mut parties {
                    party.token = if [4, 8].contains(&hour) {
                        b"mod_party".to_vec()
                    } else {
                        b"other/mod:party".to_vec()
                    };
                    party.name_default = 0;
                    party.name = format!(
                        "stored/mod:{}:{}",
                        party.country,
                        if hour == 1 { "initial" } else { "renamed" }
                    )
                    .into_bytes();
                    if party.country == 1 {
                        party.tag = count; // The naming tag belongs to the party, not its holder.
                    }
                }
                *frame.tables[TableId::PoliticalParties.index()] =
                    TableView::Complete(TableRows::PoliticalParties(parties));
            }
            state.observe(&frame).unwrap();
            if hour == 2 || hour == 7 || hour == 9 {
                publish(&mut state, &fixture); // Reloads the actual persisted continuation each time.
                let expected = match hour {
                    2 => u64::from(count),
                    7 => u64::from(count) * 2 - 1,
                    _ => u64::from(count) * 3 - 1,
                };
                assert_global_ruling_union(&state, &fixture, expected);
                if hour == 7 {
                    fixture.append_writer().commit().unwrap();
                }
            }
        }
        let saved = serde_json::to_vec(&state.ruling_changes).unwrap();
        for hour in 10..40 {
            frame.hour = hour;
            state.observe(&frame).unwrap();
            assert_eq!(
                state.ruling_changes.bytes(),
                0,
                "unchanged hours queue no global rows"
            );
        }
        assert_eq!(
            serde_json::to_vec(&state.ruling_changes).unwrap(),
            saved,
            "global continuation stores only its one event counter"
        );
        publish(&mut state, &fixture);
        assert_global_ruling_union(&state, &fixture, u64::from(count) * 3 - 1);
        assert_eq!(
            fixture
                .store
                .range_count(&fixture.key, "party-ruling-1")
                .unwrap(),
            Some(3),
            "only change-bearing commits write a global batch"
        );
    }

    #[test]
    fn prepared_party_global_ruling_stream_stably_orders_each_hour_for_single_row_pages() {
        let mut state = Prepared::default();
        let fixture = Fixture::new();
        let flag = AtomicBool::new(false);
        let change = |tag: &str, token: &str| RulingChange {
            tag: tag.into(),
            token: token.into(),
            naming: None,
            selected_tag: None,
        };
        state
            .append_ruling_changes(
                100,
                vec![change("C", "c"), change("B", "b"), change("A", "a")],
            )
            .unwrap();
        publish(&mut state, &fixture);
        state
            .append_ruling_changes(
                101,
                vec![
                    change("B", "b"),
                    change("A", "first"),
                    change("A", "second"),
                ],
            )
            .unwrap();
        publish(&mut state, &fixture);
        let mut reader =
            Reader::new(&fixture.store, &fixture.key, "party-ruling-1".into(), &flag).unwrap();
        for (ordinal, hour, tag, token) in [
            (0, 100, "A", "a"),
            (1, 100, "B", "b"),
            (2, 100, "C", "c"),
            (3, 101, "A", "first"),
            (4, 101, "A", "second"),
            (5, 101, "B", "b"),
        ] {
            let page = reader.events(&[0], hour, hour, ordinal, 1).unwrap();
            assert_eq!(
                page.rows.len(),
                1,
                "a cursor seek returns just the reached row"
            );
            let row = &page.rows[0];
            assert_eq!(row.ordinal, ordinal);
            assert_eq!(row.row["tag"], tag);
            assert_eq!(row.row["token"], token);
            assert_eq!(page.total, 3);
            assert_eq!(page.cursor, (ordinal % 3 < 2).then_some(ordinal + 1));
        }
    }

    #[test]
    fn prepared_party_global_ruling_stream_equals_per_tag_union_after_commits_resume_and_append() {
        global_ruling_stream_commits_resume_and_append(4);
    }

    fn progressive_source(blocks: i32) -> (Source, Vec<replay_frame::frame::DecodedFrame>) {
        let frames: Vec<_> = (0..blocks * 2)
            .map(|hour| {
                let value = if hour % 2 == 0 {
                    1_000_001 + i64::from(hour / 2)
                } else {
                    9_000_007
                };
                let mut frame = party_frame(hour, value, 3_500_001, "fascism", 10);
                if hour == 5
                    && let TableView::Complete(TableRows::PoliticalParties(rows)) =
                        &mut *frame.tables[TableId::PoliticalParties.index()]
                {
                    rows.push(PoliticalPartyRow {
                        country: 9001,
                        id: 3,
                        token: b"unknown/mod:party".to_vec(),
                        popularity: 17,
                        ..Default::default()
                    });
                    rows.sort_by_key(|row| (row.country, row.id));
                }
                frame
            })
            .collect();
        let mut replay =
            Replay::from_records(CAMPAIGN.into(), &[records(1, frames[..2].to_vec())]).unwrap();
        for block in 1..blocks as usize {
            replay = replay
                .append(&[dashboard_base::test_fixtures::continuation_records(
                    &replay,
                    1,
                    frames[block * 2..block * 2 + 2].to_vec(),
                )])
                .unwrap();
        }
        (source(replay), frames)
    }

    #[test]
    fn prepared_party_sampler_expands_recorded_tokens_and_keeps_missing_inputs_unknown() {
        let frame = party_frame(0, 4_000_001, 3_500_002, "fascism", 10);
        let sampled = sample(&frame, Some(&[("GER".into(), String::new())])).unwrap();
        assert_eq!(
            sampled[&("GER".into(), "value:fascism".into())].1,
            Some(4_000_001)
        );
        assert_eq!(
            sampled[&("GER".into(), "value:democratic".into())].1,
            Some(3_500_002)
        );
        assert!(sampled.keys().all(|(tag, _)| tag == "GER"));
        let absent = [
            ("absent / country".into(), "value:unknown-mod-token".into()),
            ("GER".into(), "value:missing".into()),
        ];
        assert!(
            sample(&frame, Some(&absent))
                .unwrap()
                .values()
                .all(|value| *value == (None, None))
        );
        for table in TABLES {
            let mut refused = frame.clone();
            *refused.tables[table.index()] = TableView::Unavailable {
                reason: format!("synthetic {table:?} refusal"),
            };
            assert!(sample(&refused, None).unwrap().is_empty());
            assert!(
                sample(&refused, Some(&absent))
                    .unwrap()
                    .values()
                    .all(|value| *value == (None, None))
            );
        }
        assert!(
            sample(
                &frame,
                Some(&[("GER".into(), "not-a-popularity-field".into())])
            )
            .is_err()
        );
    }

    #[test]
    fn prepared_party_progressive_popularity_draws_without_catalogue_and_zooms_thirteen_blocks_exactly()
     {
        let (source, frames) = progressive_source(15);
        let fixture = Fixture::new();
        let flag = AtomicBool::new(false);
        let context = LoadContext { content: None };
        let summary = |from, to, width, coverage: Option<&Coverage>| {
            super::query(
                &source,
                params("GER", "summary", json!({"from":from,"to":to,"width":width})),
                &context,
                &fixture.store,
                &fixture.key,
                coverage,
                &flag,
            )
            .unwrap()
        };
        let unprepared = summary(0, 29, 2, None);
        assert_eq!(
            unprepared["coverage"],
            serde_json::to_value(
                dashboard_base::prepared::prefix::extent(&source, 1, None).unwrap()
            )
            .unwrap()
        );
        assert!(!unprepared["summaries"].as_array().unwrap().is_empty());
        assert_eq!(
            unprepared["summaries"][0]["summary"]["approximate"],
            json!([{"firstHour":0,"lastHour":29}])
        );
        let zoom = summary(4, 29, 100, None);
        for line in zoom["summaries"].as_array().unwrap() {
            assert_eq!(line["summary"]["approximate"], json!([]));
            assert!(line["summary"]["runs"].is_array());
        }
        let parties = zoom["reply"]["parties"].as_array().unwrap();
        assert!(
            parties
                .iter()
                .any(|party| party["token"] == "unknown/mod:party"),
            "an exact zoom discovers a token absent from keyframes and catalogues"
        );
        let fascism = parties
            .iter()
            .find(|party| party["token"] == "fascism")
            .unwrap();
        assert!(
            fascism["runs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|run| run["valueRaw"] == "9000007"),
            "exact blocks read their non-keyframe values"
        );
        let mut state = Prepared::default();
        for frame in &frames[..2] {
            state.observe(frame).unwrap();
        }
        publish(&mut state, &fixture);
        let coverage = Coverage {
            schema: 0,
            first_hour: 0,
            last_hour: 1,
            complete: false,
        };
        let partial = summary(0, 29, 100, Some(&coverage));
        assert_eq!(partial["coverage"]["lastHour"], 1);
        assert_eq!(
            partial["summaries"][0]["summary"]["approximate"],
            json!([{"firstHour":2,"lastHour":29}])
        );
        let reached = super::query(
            &source,
            params("GER", "state", json!({"hours":[29]})),
            &context,
            &fixture.store,
            &fixture.key,
            Some(&coverage),
            &flag,
        )
        .unwrap();
        let reached = reached["reply"]["parties"]
            .as_array()
            .unwrap()
            .iter()
            .find(|party| party["token"] == "fascism")
            .unwrap();
        assert_eq!(
            reached["runs"][0]["valueRaw"], "9000007",
            "the reached unprepared hour is read from its target block"
        );
        for frame in &frames[2..16] {
            state.observe(frame).unwrap();
        }
        publish(&mut state, &fixture);
        for frame in &frames[16..] {
            state.observe(frame).unwrap();
        }
        publish(&mut state, &fixture);
        let complete = summary(
            0,
            29,
            100,
            Some(&Coverage {
                complete: true,
                last_hour: 29,
                ..coverage
            }),
        );
        assert_eq!(
            complete["summaries"][0]["summary"]["approximate"],
            json!([])
        );
        let legacy = super::super::scan(source.replay_ref(), 1, "GER", &flag).unwrap();
        assert_eq!(
            complete["reply"]["parties"],
            serde_json::to_value(legacy.parties.values().collect::<Vec<_>>()).unwrap()
        );
    }

    #[test]
    fn prepared_party_tail_only_duplicate_keeps_first_sample_and_returns_the_legacy_overlap_refusal()
     {
        let (_, mut frames) = progressive_source(2);
        if let TableView::Complete(TableRows::PoliticalParties(rows)) =
            &mut *frames[3].tables[TableId::PoliticalParties.index()]
        {
            let mut duplicate = rows
                .iter()
                .find(|row| row.country == 9001 && row.token == b"fascism")
                .unwrap()
                .clone();
            duplicate.id = 3;
            duplicate.popularity = -7;
            rows.push(duplicate);
            rows.sort_by_key(|row| (row.country, row.id));
        }
        let source = self::source(
            Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap(),
        );
        let fixture = Fixture::new();
        let mut state = Prepared::default();
        for frame in &frames[..2] {
            state.observe(frame).unwrap();
        }
        publish(&mut state, &fixture);
        assert!(!state.metadata.duplicate.contains_key("GER"));
        let key = ("GER".into(), "value:fascism".into());
        let mut duplicates = BTreeMap::new();
        assert_eq!(
            sample_with_duplicates(&frames[3], None, &mut duplicates).unwrap()[&key].1,
            Some(9_000_007)
        );
        assert_eq!(duplicates[&key], (3, 9_000_007));
        let coverage = Coverage {
            schema: 0,
            first_hour: 0,
            last_hour: 1,
            complete: false,
        };
        for mode in ["summary", "state"] {
            let ask = || {
                super::query(
                    &source,
                    params("GER", mode, json!({"from":2,"to":3,"width":10,"hours":[3]})),
                    &LoadContext { content: None },
                    &fixture.store,
                    &fixture.key,
                    Some(&coverage),
                    &AtomicBool::new(false),
                )
                .unwrap()
            };
            let reply = ask();
            let runs = reply["reply"]["parties"]
                .as_array()
                .unwrap()
                .iter()
                .find(|party| party["token"] == "fascism")
                .unwrap()["runs"]
                .as_array()
                .unwrap();
            assert!(
                runs.windows(2)
                    .any(|rows| rows[0]["lastHour"].as_i64().unwrap()
                        >= rows[1]["firstHour"].as_i64().unwrap()),
                "the existing UI validator must refuse a tail-only duplicate, never display a collapsed value"
            );
            if mode == "summary" {
                let before = dashboard_base::prepared::query::observations::usage(
                    &source,
                    "party-history:popularity",
                );
                assert_eq!(before.2, 1, "one sampled block fold");
                let (again, refused) = progressive::without_block_decodes(ask);
                assert!(!refused);
                assert_eq!(
                    serde_json::to_vec(&again).unwrap(),
                    serde_json::to_vec(&reply).unwrap()
                );
                let after = dashboard_base::prepared::query::observations::usage(
                    &source,
                    "party-history:popularity",
                );
                assert_eq!(after.2, before.2, "the duplicate block was folded again");
                assert!(after.0 > 0 && after.0 <= after.1);
            }
        }
    }

    #[test]
    fn prepared_party_duplicate_tokens_preserve_the_legacy_widget_refusal() {
        let mut frame = party_frame(100, 1, 2, "fascism", 10);
        if let TableView::Complete(TableRows::PoliticalParties(rows)) =
            &mut *frame.tables[TableId::PoliticalParties.index()]
        {
            let mut duplicate = rows
                .iter()
                .find(|row| row.country == 9001 && row.token == b"democratic")
                .unwrap()
                .clone();
            duplicate.id = 3;
            rows.push(duplicate);
            rows.sort_by_key(|row| (row.country, row.id));
        }
        let source = source(
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame.clone()])]).unwrap(),
        );
        let old =
            super::super::scan(source.replay_ref(), 1, "GER", &AtomicBool::new(false)).unwrap();
        assert_eq!(old.parties["democratic"].runs.len(), 2);
        let mut state = Prepared::default();
        let fixture = Fixture::new();
        state.observe(&frame).unwrap();
        publish(&mut state, &fixture);
        let flag = AtomicBool::new(false);
        let mut reader =
            Reader::new(&fixture.store, &fixture.key, "party-series-1".into(), &flag).unwrap();
        let token_key = field(&state.keys, "GER", "value:democratic").unwrap();
        assert_eq!(
            reader.runs(token_key, 100, 100).unwrap().len(),
            2,
            "the original duplicate rows remain in shared storage"
        );
        for mode in ["history", "state", "summary"] {
            let result = query(
                &source,
                params(
                    "GER",
                    mode,
                    json!({"hours":[100],"from":100,"to":100,"width":100}),
                ),
                &LoadContext { content: None },
                &fixture.store,
                &fixture.key,
                &AtomicBool::new(false),
            );
            let result = result.unwrap();
            let parties = result["reply"]["parties"].as_array().unwrap();
            let runs = parties
                .iter()
                .find(|party| party["token"] == "democratic")
                .unwrap()["runs"]
                .as_array()
                .unwrap();
            assert!(
                runs.windows(2)
                    .any(|rows| rows[0]["lastHour"].as_i64().unwrap()
                        >= rows[1]["firstHour"].as_i64().unwrap()),
                "the production UI validator sees the same overlap"
            );
        }
    }
}
