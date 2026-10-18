//! The country balance observer: native holders, unknown hours and exact net drops.
use crate::metadata::{CountryKeys, TaggedCountry, tagged_countries};
use crate::prepared::{
    frame_changes::FrameChanges,
    series::{self, Accumulator, Run},
};
use crate::products::country_measure::{BalanceDrop, MEASURE_DECIMALS, Measure};
use replay_data::{rows::CountryLinkRow, rows::CountryRow, rows::tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io, sync::atomic::AtomicBool};
use ts_rs::TS;

pub mod query;

/// The complete recorded tables needed by the stateless balance sampler.
pub const TABLES: &[replay_data::rows::TableId] = &[
    replay_data::rows::TableId::Campaign,
    replay_data::rows::TableId::Countries,
    replay_data::rows::TableId::CountryLinks,
];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct MeasureRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub value_raw: String,
    pub slot: u32,
    pub country_id: u32,
    pub starts_segment: bool,
    pub segment_first_hour: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub read_hour: Option<i32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MeasureActor {
    pub actor: String,
    pub tag: String,
    pub measure: String,
    pub first_hour: i32,
    pub last_hour: i32,
    pub run_count: String,
    pub drop_count: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureHolder {
    pub slot: u32,
    pub country_id: u32,
    #[serde(default)]
    pub read_hour: Option<i32>,
    #[serde(default)]
    pub segment_first_hour: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experience: Option<crate::products::country_experience::ExperienceRead>,
}
impl MeasureRun {
    pub fn from_segment(run: Run, segment_first_hour: i32, holder: MeasureHolder) -> Self {
        Self {
            first_hour: run.first_hour,
            last_hour: run.last_hour,
            value_raw: run.value.to_string(),
            starts_segment: run.break_before,
            slot: holder.slot,
            country_id: holder.country_id,
            segment_first_hour: holder.segment_first_hour.unwrap_or(segment_first_hour),
            read_hour: holder.read_hour,
        }
    }
}
/// Stateless extraction shared by preparation and direct progressive queries.
/// Selection addresses recorded tags and measure labels, never catalogue keys.
#[allow(clippy::type_complexity)]
pub fn sample(
    frame: &DecodedFrame,
    requested: Option<&[(String, String)]>,
) -> io::Result<BTreeMap<(String, String), (Option<(u32, u32)>, Option<i128>)>> {
    if requested.is_some_and(|rows| {
        rows.iter()
            .any(|(_, label)| !["politicalPower", "commandPower"].contains(&label.as_str()))
    }) {
        return Err(invalid("Unknown country balance sample measure"));
    }
    let wanted = requested.map(|rows| {
        rows.iter()
            .map(|(tag, label)| (tag.as_str(), label.as_str()))
            .collect::<std::collections::BTreeSet<_>>()
    });
    let (rows, _) = sample_rows(
        frame
            .complete::<tables::Countries>()
            .ok()
            .zip(frame.complete::<tables::CountryLinks>().ok()),
        #[cfg(any(test, feature = "test-support"))]
        &mut crate::prepared::observer_tests::Work::default(),
    );
    let mut result = BTreeMap::new();
    for (tag, (identity, values)) in rows {
        for (label, value) in ["politicalPower", "commandPower"].into_iter().zip(values) {
            if wanted
                .as_ref()
                .is_none_or(|rows| rows.contains(&(tag.as_str(), label)))
            {
                result.insert((tag.clone(), label.to_owned()), (identity, value));
            }
        }
    }
    if let Some(requested) = requested {
        for key in requested {
            result.entry(key.clone()).or_insert((None, None));
        }
    }
    Ok(result)
}

type BalanceSample = (Option<(u32, u32)>, [Option<i128>; 2]);
fn sample_country(row: &CountryRow, slot: u32) -> BalanceSample {
    (
        Some((slot, row.id)),
        [
            row.political_power_raw.map(i128::from),
            Some(i128::from(row.command_power_raw)),
        ],
    )
}
fn sample_rows(
    rows: Option<(&[CountryRow], &[CountryLinkRow])>,
    #[cfg(any(test, feature = "test-support"))] work: &mut crate::prepared::observer_tests::Work,
) -> (BTreeMap<String, BalanceSample>, (u64, u64)) {
    let mut result = BTreeMap::new();
    let (mut unresolved, mut unnamed) = (0, 0);
    if let Some((countries, links)) = rows {
        for (tag, lookup) in tagged_countries(countries, links) {
            #[cfg(any(test, feature = "test-support"))]
            {
                work.row_visits += 1;
            }
            if !matches!(lookup, TaggedCountry::Found(..)) {
                unresolved += 1;
            }
            let Ok(tag) = std::str::from_utf8(tag) else {
                unnamed += u64::from(matches!(lookup, TaggedCountry::Found(..)));
                continue;
            };
            let sample = match lookup {
                TaggedCountry::Found(row, slot) => sample_country(row, slot),
                _ => (None, [None; 2]),
            };
            result.insert(tag.to_owned(), sample);
        }
    }
    (result, (unresolved, unnamed))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct BalanceAccumulator {
    actor: u64,
    first_hour: i32,
    value: Option<i128>,
}
impl BalanceAccumulator {
    fn new(actor: u64, first_hour: i32) -> Self {
        Self {
            actor,
            first_hour,
            value: None,
        }
    }
    #[cfg_attr(any(test, feature = "test-support"), allow(clippy::too_many_arguments))]
    fn observe(
        &mut self,
        hour: i32,
        identity: Option<(u32, u32)>,
        amount: Option<i128>,
        identity_contiguous: bool,
        values: &mut Accumulator,
        segments: &mut Accumulator,
        #[cfg(any(test, feature = "test-support"))]
        work: &mut crate::prepared::observer_tests::Work,
    ) -> io::Result<()> {
        let previous = self.value;
        let contiguous = identity_contiguous && previous.is_some();
        let present = amount.zip(identity);
        if (present.is_none() && previous.is_none()) || (contiguous && previous == amount) {
            return Ok(());
        }
        if contiguous
            && let (Some(before), Some(after)) = (previous, amount)
            && after < before
        {
            #[cfg(any(test, feature = "test-support"))]
            {
                work.emitted += 1;
            }
            values.event(
                self.actor,
                hour,
                &BalanceDrop {
                    hour,
                    before_raw: before.to_string(),
                    after_raw: after.to_string(),
                },
            )?;
        }
        if let Some((amount, (slot, country_id))) = present {
            if !contiguous {
                #[cfg(any(test, feature = "test-support"))]
                {
                    work.emitted += 1;
                }
                segments.event(
                    self.actor,
                    hour,
                    &MeasureHolder {
                        slot,
                        country_id,
                        read_hour: None,
                        segment_first_hour: None,
                        experience: None,
                    },
                )?;
            }
            values.observe(self.actor, hour, Some(amount), !contiguous);
        } else {
            values.observe(self.actor, hour, None, false);
        }
        #[cfg(any(test, feature = "test-support"))]
        {
            work.emitted += 1;
        }
        self.value = present.map(|(amount, _)| amount);
        Ok(())
    }
    fn metadata(
        &self,
        tag: &str,
        measure: Measure,
        last_hour: i32,
        values: &Accumulator,
    ) -> MeasureActor {
        let state = values.state(self.actor);
        MeasureActor {
            actor: self.actor.to_string(),
            tag: tag.to_owned(),
            measure: match measure {
                Measure::PoliticalPower => "politicalPower",
                Measure::CommandPower => "commandPower",
            }
            .into(),
            first_hour: self.first_hour,
            last_hour,
            run_count: state.map_or(0, |state| state.runs).to_string(),
            drop_count: state.map_or(0, |state| state.events).to_string(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct CountryAccumulator {
    political: BalanceAccumulator,
    command: BalanceAccumulator,
}
#[derive(Serialize, Deserialize)]
pub struct MeasureAccumulator {
    schema: u32,
    first_hour: i32,
    next_hour: i64,
    next_actor: u64,
    countries: BTreeMap<String, CountryAccumulator>,
    holders: BTreeMap<String, (u32, u32)>,
    #[serde(skip)]
    keys: Option<CountryKeys>,
    #[serde(skip)]
    diagnostic_counts: (u64, u64),
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    pub work: crate::prepared::observer_tests::Work,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    pub test_changes: Option<crate::prepared::frame_changes::FrameChangeTracker>,
    pub unresolved_countries: u64,
    pub unnamed_countries: u64,
    batches: [Accumulator; 2],
    segments: Accumulator,
}
impl MeasureAccumulator {
    pub fn new(first_hour: i32) -> Self {
        Self {
            schema: Self::SCHEMA,
            first_hour,
            next_hour: i64::from(first_hour),
            next_actor: 0,
            countries: BTreeMap::new(),
            holders: BTreeMap::new(),
            keys: None,
            diagnostic_counts: (0, 0),
            #[cfg(any(test, feature = "test-support"))]
            work: Default::default(),
            #[cfg(any(test, feature = "test-support"))]
            test_changes: None,
            unresolved_countries: 0,
            unnamed_countries: 0,
            batches: Default::default(),
            segments: Accumulator::default(),
        }
    }
    pub const SCHEMA: u32 = 6;
    pub fn validate_checkpoint(&self) -> io::Result<()> {
        if self.schema != Self::SCHEMA
            || self.next_hour < i64::from(self.first_hour)
            || self.next_actor != self.countries.len() as u64 * 2
        {
            return Err(invalid("prepared balance checkpoint schema mismatch"));
        }
        Ok(())
    }
    pub fn next_hour(&self) -> i64 {
        self.next_hour
    }
    pub fn buffered_bytes(&self) -> usize {
        self.batches.iter().map(Accumulator::bytes).sum::<usize>() + self.segments.bytes()
    }
    pub fn observe_changes(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        self.observe_inputs(
            frame.hour,
            frame
                .complete::<tables::Countries>()
                .ok()
                .zip(frame.complete::<tables::CountryLinks>().ok()),
            changes.rows_changed[replay_data::rows::TableId::Countries.index()]
                || changes.rows_changed[replay_data::rows::TableId::CountryLinks.index()],
            changes.rows_changed[replay_data::rows::TableId::CountryLinks.index()],
        )
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn observe(&mut self, frame: &DecodedFrame) -> io::Result<()> {
        let changes = self
            .test_changes
            .get_or_insert_with(crate::prepared::frame_changes::FrameChangeTracker::new)
            .observe(frame, replay_data::format::FormatVersion::CURRENT);
        self.observe_changes(frame, &changes)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn observe_rows(
        &mut self,
        hour: i32,
        rows: Option<(&[CountryRow], &[CountryLinkRow])>,
    ) -> io::Result<()> {
        self.observe_inputs(hour, rows, true, true)
    }

    fn observe_inputs(
        &mut self,
        hour: i32,
        rows: Option<(&[CountryRow], &[CountryLinkRow])>,
        changed: bool,
        links_changed: bool,
    ) -> io::Result<()> {
        if i64::from(hour) < self.next_hour {
            return Err(invalid("measure frame hours reversed"));
        }
        let contiguous = self.next_hour == i64::from(hour);
        for batch in &mut self.batches {
            if !contiguous {
                batch.through(self.next_hour as i32);
            }
            batch.advance(hour)?;
        }
        if !contiguous {
            self.segments.through(self.next_hour as i32);
        }
        self.segments.through(hour);
        if changed || !contiguous || self.next_hour == i64::from(self.first_hour) {
            let same_keys = contiguous
                && !links_changed
                && rows.is_some_and(|(countries, _)| {
                    self.keys.as_ref().is_some_and(|keys| {
                        #[cfg(any(test, feature = "test-support"))]
                        let before = keys.row_visits();
                        let same = keys.same(countries);
                        #[cfg(any(test, feature = "test-support"))]
                        {
                            self.work.row_visits += keys.row_visits() - before;
                        }
                        same
                    })
                });
            let observed = if same_keys {
                // CountryKeys proves row identities and ordering unchanged;
                // the existing native holder projection supplies the join.
                let mut observed = BTreeMap::new();
                for row in rows.expect("complete measure inputs").0 {
                    #[cfg(any(test, feature = "test-support"))]
                    {
                        self.work.row_visits += 1;
                    }
                    let Ok(tag) = std::str::from_utf8(&row.tag) else {
                        continue;
                    };
                    let sample = observed.entry(tag.to_owned()).or_insert((None, [None; 2]));
                    if sample.0.is_none()
                        && self.holders.get(tag) == Some(&(row.canonical_slot, row.id))
                    {
                        *sample = sample_country(row, row.canonical_slot);
                    }
                }
                observed
            } else {
                let (observed, counts) = sample_rows(
                    rows,
                    #[cfg(any(test, feature = "test-support"))]
                    &mut self.work,
                );
                self.diagnostic_counts = counts;
                self.keys = rows.map(|(countries, _)| CountryKeys::read(countries));
                observed
            };
            // Only the current native roster is compared for removals; historic
            // actors remain in the catalogue without being visited per frame.
            let removed: Vec<_> = if same_keys {
                Vec::new()
            } else {
                self.holders
                    .keys()
                    .filter(|tag| {
                        #[cfg(any(test, feature = "test-support"))]
                        {
                            self.work.history_visits += 1;
                        }
                        observed
                            .get(tag.as_str())
                            .is_none_or(|(identity, _)| identity.is_none())
                    })
                    .cloned()
                    .collect()
            };
            for tag in removed {
                self.holders.remove(&tag);
                let country = self
                    .countries
                    .get_mut(&tag)
                    .expect("registered measure holder");
                country.political.observe(
                    hour,
                    None,
                    None,
                    false,
                    &mut self.batches[0],
                    &mut self.segments,
                    #[cfg(any(test, feature = "test-support"))]
                    &mut self.work,
                )?;
                country.command.observe(
                    hour,
                    None,
                    None,
                    false,
                    &mut self.batches[1],
                    &mut self.segments,
                    #[cfg(any(test, feature = "test-support"))]
                    &mut self.work,
                )?;
            }
            for (tag, (identity, [political, command])) in &observed {
                let Some(identity) = *identity else { continue };
                let actor = self.next_actor;
                let country = match self.countries.entry(tag.clone()) {
                    std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        self.next_actor = self
                            .next_actor
                            .checked_add(2)
                            .ok_or_else(|| invalid("measure actor ordinal overflow"))?;
                        entry.insert(CountryAccumulator {
                            political: BalanceAccumulator::new(actor, hour),
                            command: BalanceAccumulator::new(actor + 1, hour),
                        })
                    }
                };
                let old = if same_keys {
                    None
                } else {
                    self.holders.get(tag).copied()
                };
                let continues = contiguous && (same_keys || old == Some(identity));
                country.political.observe(
                    hour,
                    Some(identity),
                    *political,
                    continues,
                    &mut self.batches[0],
                    &mut self.segments,
                    #[cfg(any(test, feature = "test-support"))]
                    &mut self.work,
                )?;
                country.command.observe(
                    hour,
                    Some(identity),
                    *command,
                    continues,
                    &mut self.batches[1],
                    &mut self.segments,
                    #[cfg(any(test, feature = "test-support"))]
                    &mut self.work,
                )?;
                if !same_keys && old != Some(identity) {
                    self.holders.insert(tag.clone(), identity);
                }
            }
        }
        // Equal carried inputs repeat the oracle's diagnostics for this hour,
        // without rebuilding their tag/link join or visiting native rows again.
        self.unresolved_countries += self.diagnostic_counts.0;
        self.unnamed_countries += self.diagnostic_counts.1;
        self.next_hour = i64::from(hour) + 1;
        Ok(())
    }
    pub fn catalogue(&self) -> Vec<MeasureActor> {
        let mut actors = Vec::new();
        for (tag, country) in &self.countries {
            actors.push(country.political.metadata(
                tag,
                Measure::PoliticalPower,
                (self.next_hour - 1) as i32,
                &self.batches[0],
            ));
            actors.push(country.command.metadata(
                tag,
                Measure::CommandPower,
                (self.next_hour - 1) as i32,
                &self.batches[1],
            ));
        }
        actors
    }
    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        for (index, values) in self.batches.iter_mut().enumerate() {
            values.flush(
                writer,
                &format!("measure-{}-{branch}", if index == 0 { "pp" } else { "cp" }),
                MEASURE_DECIMALS,
                cancelled,
            )?;
        }
        self.segments
            .flush(writer, &format!("measure-segments-{branch}"), 0, cancelled)?;
        series::write_catalogue(
            writer,
            "measure",
            branch,
            &self
                .countries
                .iter()
                .flat_map(|(tag, country)| {
                    [
                        (
                            series::field_key(tag, "politicalPower"),
                            country.political.actor,
                        ),
                        (
                            series::field_key(tag, "commandPower"),
                            country.command.actor,
                        ),
                    ]
                })
                .collect(),
            &self.catalogue(),
        )
    }
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests;
