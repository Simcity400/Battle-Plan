//! Pair values use shared exact series; sparse events retain the production scan's
//! run operands, attempt dates and tags. The checkpoint keeps only open scan rows.
use super::{
    CipherObservation, DecryptionCoverage, DecryptionPair, DecryptionReply, DecryptionRun,
    DecryptionTagRun, Scan,
};
use dashboard_base::prepared::{
    frame_changes::FrameChanges,
    series::{self, Reader},
};
use dashboard_text::{cryptology, metadata::check_cancel};
use replay::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::AtomicBool,
};

#[derive(Deserialize, Serialize)]
struct Observation {
    run: DecryptionRun,
    tags: DecryptionTagRun,
    broken: Option<CipherObservation>,
}

#[derive(Default, Deserialize, Serialize)]
pub struct Accumulator {
    scan: Scan,
    values: series::Accumulator,
    keys: BTreeMap<String, u64>,
    first: Option<i32>,
    last: Option<i32>,
    content_warnings: Vec<String>,
    active: BTreeSet<(u32, u32)>,
    next_clock: Option<i32>,
    #[serde(skip)]
    configured: bool,
    #[cfg(test)]
    #[serde(skip)]
    test_changes: Option<dashboard_base::prepared::frame_changes::FrameChangeTracker>,
    #[cfg(test)]
    #[serde(skip)]
    test_countries: super::super::operative_rows::CountryInputs,
}

#[derive(Deserialize, Serialize)]
struct Metadata {
    first: i32,
    last: i32,
    warnings: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Params {
    pub branch_id: String,
    #[serde(default)]
    pub pairs: Vec<(String, String)>,
    pub first_hour: Option<i32>,
    pub last_hour: Option<i32>,
}

impl Accumulator {
    #[cfg(test)]
    fn observe_with_defines(
        &mut self,
        frame: &DecodedFrame,
        defines: Result<cryptology::Defines, &str>,
    ) -> io::Result<()> {
        let changes = self
            .test_changes
            .get_or_insert_with(dashboard_base::prepared::frame_changes::FrameChangeTracker::new)
            .observe(frame, replay::format::FormatVersion::default());
        self.test_countries.update(frame, Some(&changes));
        let countries = std::mem::take(&mut self.test_countries);
        let result = self.observe_bound(frame, &changes, &countries, defines, &[]);
        self.test_countries = countries;
        result
    }

    pub fn observe_bound(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        countries: &super::super::operative_rows::CountryInputs,
        defines: Result<cryptology::Defines, &str>,
        warnings: &[String],
    ) -> io::Result<()> {
        use replay::rows::TableId;
        let hour = frame.hour;
        let previous = self.last;
        if !self.configured {
            self.content_warnings = warnings.to_vec();
            self.configured = true;
        }
        self.first.get_or_insert(hour);
        self.last = Some(hour);
        self.values.advance(hour)?;
        let inputs_changed = countries.changed
            || previous.is_none_or(|last| last.checked_add(1) != Some(hour))
            || [TableId::Cryptology, TableId::RaidIntelInputs]
                .iter()
                .any(|table| changes.rows_changed[table.index()]);
        let previous_coverage = self.scan.coverage.last().cloned();
        if !inputs_changed && self.next_clock.is_none_or(|at| hour < at) {
            self.scan
                .cover(hour, previous_coverage.and_then(|coverage| coverage.reason));
            return Ok(());
        }
        // Extend only live native operands, once when their inputs are reached.
        // Closed pairs stay on disk and never participate in an hourly sweep.
        if let Some(previous) = previous {
            for key in &self.active {
                #[cfg(test)]
                super::super::intelligence_prepared::work(|work| work.history_visits += 1);
                let pair = &mut self.scan.pairs.get_mut(key).expect("live cipher pair").pair;
                if let Some(run) = pair.runs.last_mut() {
                    run.last_hour = previous;
                }
                if let Some(tags) = pair.tag_runs.last_mut() {
                    tags.last_hour = previous;
                }
            }
        }
        let observed = self
            .scan
            .frame_countries(frame, defines, countries, Some(changes))?;
        let active: BTreeSet<_> = observed.iter().map(|row| row.key).collect();
        for removed in self.active.difference(&active) {
            let key = self.keys[&serde_json::to_string(removed)?];
            self.values.observe(key, hour, None, false);
            self.values.observe(key + 1, hour, None, false);
        }
        self.active = active;
        self.next_clock = None;
        for observed in observed {
            let name = serde_json::to_string(&observed.key)?;
            let next = u64::try_from(self.keys.len())
                .ok()
                .and_then(|count| count.checked_add(1))
                .and_then(|count| count.checked_mul(2))
                .ok_or_else(|| io::Error::other("Too many cipher pairs"))?;
            let key = *self.keys.entry(name).or_insert(next);
            let state = self
                .scan
                .pairs
                .get_mut(&observed.key)
                .expect("observed cipher pair");
            let run = state.pair.runs.last().expect("observed cipher run");
            let mut observation = Observation {
                run: run.clone(),
                tags: state
                    .pair
                    .tag_runs
                    .last()
                    .expect("observed cipher tags")
                    .clone(),
                broken: state.pair.breaks.last().cloned(),
            };
            // Extension changes only the endpoint; it is represented by exact
            // value coverage, rather than another copy of the event operands.
            observation.run.last_hour = observation.run.first_hour;
            observation.tags.last_hour = observation.tags.first_hour;
            for (key, value) in [
                (key, run.amount_at(hour).map(i128::from)),
                (key + 1, run.bonus_days_at(hour).map(i128::from)),
            ] {
                if observed.starts
                    || self
                        .values
                        .state(key)
                        .and_then(|state| state.last)
                        .map(|run| run.value)
                        != value
                {
                    self.values.observe(key, hour, value, observed.starts);
                }
            }
            // A learned daily slope must still agree with every plateau. An
            // unchanged captured row can invalidate it at its next native phase.
            if run.step_raw_per_day != "0" || run.bonus_days_step_per_day != 0 {
                let days = (i64::from(hour) - i64::from(run.first_hour)) / 24 + 1;
                if let Ok(at) = i32::try_from(i64::from(run.first_hour) + days * 24) {
                    self.next_clock = Some(self.next_clock.map_or(at, |old| old.min(at)));
                }
            }
            if observed.changed {
                #[cfg(test)]
                super::super::intelligence_prepared::work(|work| work.emitted += 1);
                self.values.event(key, hour, &observation)?;
            }
            // The same scan supplies attempt continuity after a commit. Closed
            // histories are already in series files and never enter its checkpoint.
            if state.pair.runs.len() > 1 {
                state.pair.runs.drain(..state.pair.runs.len() - 1);
            }
            if state.pair.tag_runs.len() > 1 {
                state.pair.tag_runs.drain(..state.pair.tag_runs.len() - 1);
            }
            if state.pair.breaks.len() > 1 {
                state.pair.breaks.drain(..state.pair.breaks.len() - 1);
            }
        }
        let coverage = self.scan.coverage.last().expect("observed cipher coverage");
        let starts = previous_coverage
            .as_ref()
            .is_none_or(|previous| previous.first_hour != coverage.first_hour);
        if starts {
            self.values
                .observe(0, hour, Some(i128::from(coverage.complete)), true);
            self.values.event(0, hour, coverage)?;
        }
        if self.scan.coverage.len() > 1 {
            self.scan.coverage.drain(..self.scan.coverage.len() - 1);
        }
        Ok(())
    }

    pub fn buffered_bytes(&self) -> usize {
        self.values.bytes()
    }

    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        if let Some(last) = self.last {
            for key in &self.active {
                let pair = &mut self.scan.pairs.get_mut(key).expect("open cipher pair").pair;
                if let Some(run) = pair.runs.last_mut() {
                    run.last_hour = last;
                }
                if let Some(tags) = pair.tag_runs.last_mut() {
                    tags.last_hour = last;
                }
            }
        }
        self.values
            .flush(writer, &format!("decryption-{branch}-series"), 0, cancelled)?;
        if let Some((first, last)) = self.first.zip(self.last) {
            let mut warnings = self
                .scan
                .refusals
                .diagnostic_lines("cipher entries affected");
            warnings.extend(self.content_warnings.iter().cloned());
            series::write_catalogue(
                writer,
                "decryption",
                branch,
                &self.keys,
                &Metadata {
                    first,
                    last,
                    warnings,
                },
            )?;
        }
        series::write_reply(writer, &format!("decryption-{branch}"), 0, self)
    }

    pub fn read(reader: impl io::Read) -> io::Result<Self> {
        Ok(serde_json::from_reader(zstd::stream::Decoder::new(
            reader,
        )?)?)
    }
}

/// A range's final supplying run can precede its right boundary when a pair
/// disappears. Shared navigation finds that boundary without visiting the tail.
fn last_covered(
    reader: &mut Reader<'_>,
    key: u64,
    first: i32,
    last: i32,
) -> io::Result<Option<i32>> {
    if reader.value_at(key, last)?.is_some() {
        return Ok(Some(last));
    }
    let Some(boundary) = reader.next_change(key, f64::from(last) + 1.0, false)? else {
        return Ok(None);
    };
    for hour in [Some(boundary), boundary.checked_sub(1)]
        .into_iter()
        .flatten()
    {
        if hour < first {
            continue;
        }
        if let Some(run) = reader.value_at(key, hour)? {
            return Ok(Some(run.last_hour.min(last)));
        }
    }
    Ok(None)
}

fn pair_reply(
    reader: &mut Reader<'_>,
    key: u64,
    pair: (u32, u32),
    first: i32,
    last: i32,
    cancelled: &AtomicBool,
) -> io::Result<DecryptionPair> {
    let mut observations = Vec::new();
    if let Some((_, row)) = reader.latest_event(key, first)? {
        observations.push(serde_json::from_value::<Observation>(row)?);
    }
    if let Some(from) = first.checked_add(1).filter(|from| *from <= last) {
        let mut cursor = 0;
        loop {
            check_cancel(cancelled)?;
            let page = reader.events(&[key], from, last, cursor, 256)?;
            for row in page.rows {
                observations.push(serde_json::from_value::<Observation>(row.row)?);
            }
            let Some(next) = page.cursor else {
                break;
            };
            cursor = next;
        }
    }
    let mut runs = BTreeMap::new();
    let mut tags = BTreeMap::new();
    let mut breaks = BTreeMap::new();
    for observation in observations {
        runs.insert(observation.run.first_hour, observation.run);
        tags.insert(observation.tags.first_hour, observation.tags);
        if let Some(broken) = observation.broken {
            breaks.insert(broken.hour, broken);
        }
    }
    let mut runs: Vec<_> = runs.into_values().collect();
    for index in 0..runs.len() {
        let end = runs.get(index + 1).map_or(last, |next| next.first_hour - 1);
        runs[index].last_hour = last_covered(reader, key, runs[index].first_hour, end)?
            .unwrap_or(runs[index].first_hour - 1);
    }
    runs.retain(|run| run.last_hour >= first && run.first_hour <= last);
    let mut tags: Vec<_> = tags.into_values().collect();
    for index in 0..tags.len() {
        let end = tags.get(index + 1).map_or(last, |next| next.first_hour - 1);
        tags[index].last_hour = last_covered(reader, key, tags[index].first_hour, end)?
            .unwrap_or(tags[index].first_hour - 1);
    }
    tags.retain(|run| run.last_hour >= first && run.first_hour <= last);
    Ok(DecryptionPair {
        owner_id: pair.0.to_string(),
        target_id: pair.1.to_string(),
        runs,
        tag_runs: tags,
        breaks: breaks.into_values().collect(),
    })
}

pub fn query(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    params: &Params,
    cancelled: &AtomicBool,
) -> io::Result<Option<DecryptionReply>> {
    if params.branch_id.parse::<u64>().ok() != Some(branch) {
        return Err(dashboard_core::invalid(
            "Cipher parameters do not match the dispatched branch",
        ));
    }
    let Some((keys, metadata)) =
        series::read_catalogue::<Metadata>(store, key, "decryption", branch)?
    else {
        return Ok(None);
    };
    let first = params.first_hour.unwrap_or(metadata.first);
    let last = params.last_hour.unwrap_or(metadata.last);
    if first > last {
        return Err(dashboard_core::invalid("Invalid cipher query range"));
    }
    let mut selected = BTreeMap::new();
    for (name, key) in keys {
        let pair: (u32, u32) = serde_json::from_str(&name)?;
        if params.pairs.is_empty()
            || params.pairs.iter().any(|requested| {
                requested.0 == pair.0.to_string() && requested.1 == pair.1.to_string()
            })
        {
            selected.insert(pair, key);
        }
    }
    let mut reader = Reader::new(store, key, format!("decryption-{branch}-series"), cancelled)?;
    let mut pairs = Vec::new();
    for (pair, key) in selected {
        check_cancel(cancelled)?;
        let pair = pair_reply(&mut reader, key, pair, first, last, cancelled)?;
        if !pair.runs.is_empty() {
            pairs.push(pair);
        }
    }
    let mut coverage = Vec::new();
    let mut hour = first;
    while hour <= last {
        check_cancel(cancelled)?;
        if let Some(run) = reader.value_at(0, hour)? {
            let (_, row) = reader
                .latest_event(0, hour)?
                .ok_or_else(|| dashboard_core::invalid("Missing cipher coverage event"))?;
            let mut entry: DecryptionCoverage = serde_json::from_value(row)?;
            entry.first_hour = run.first_hour;
            entry.last_hour = run.last_hour.min(last);
            let next = entry.last_hour.checked_add(1);
            coverage.push(entry);
            let Some(next) = next else {
                break;
            };
            hour = next;
        } else {
            let Some(next) = reader.next_change(0, f64::from(hour), true)? else {
                break;
            };
            if next <= hour {
                return Err(dashboard_core::invalid("Invalid cipher coverage boundary"));
            }
            hour = next;
        }
    }
    let reply = DecryptionReply {
        branch_id: branch.to_string(),
        pairs,
        coverage,
        warnings: metadata.warnings,
    };
    super::check_reply_budget(&reply)?;
    Ok(Some(reply))
}

#[cfg(test)]
mod tests;
