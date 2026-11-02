//! The naval scan kept in order from projected hours ([`super::Hour`]).

use super::{Combats, Countries, Hour, Results, Sources};
use crate::products::{
    naval_battles::{Kind, ResultInput, Scan, incomplete, span, windows},
    naval_losses, ships_sunk,
};
use dashboard_base::{prepared::pages::Pages, products::battles::BattleWindow, source::Source};
use dashboard_series::Writer;
use replay::container::Replay;
use replay_data::rows::NavalResultSourceRow;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    io,
    sync::atomic::AtomicBool,
};

/// What the scan keeps of projected hours beyond the row path's own state.
#[derive(Default, Serialize, Deserialize)]
pub(in crate::products::naval_battles) struct Carried {
    /// Each result the last result read held.
    present: BTreeSet<u64>,
    /// The held results and present combats whose last sample is incomplete.
    incomplete_results: BTreeSet<u64>,
    incomplete_combats: BTreeSet<u64>,
    /// Whether the last result read stands, as the row path's held result views do.
    views: bool,
    /// The results whose source observation changed since the last result read.
    observed: BTreeSet<u64>,
    tags: bool,
    /// The rows the last result read refused: member sides, country lists, classifications.
    refused: [usize; 3],
    /// The result sources last read.
    #[serde(
        serialize_with = "crate::products::naval_battles::serialize_rows",
        deserialize_with = "crate::products::naval_battles::deserialize_rows"
    )]
    sources: Vec<NavalResultSourceRow>,
}

impl Scan {
    /// [`Self::observe_prepared`] then [`Self::observe_details_read`] of a projected hour.
    pub fn observe_projected(
        &mut self,
        mut hour: Hour,
        source: &Source,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        {
            let _cpu = ::replay_frame::cpu::scope("naval emit census", None);
            self.observe_census(&hour)?;
        }
        let _cpu = ::replay_frame::cpu::scope("naval emit details", None);
        self.observe_projected_details(source, &mut hour, cancelled)
    }

    fn observe_census(&mut self, hour: &Hour) -> io::Result<()> {
        let at = hour.hour;
        self.available |= !matches!(hour.combats, Combats::Unavailable)
            || !matches!(hour.results, Results::Unavailable);
        self.hour = at;
        span(&mut self.recorded_hours, at);
        let previous_hour = self.last_hour;
        let contiguous = previous_hour.is_some_and(|last| last.checked_add(1) == Some(at));
        if !contiguous {
            self.extend_results();
            self.combats.clear();
            self.projected.incomplete_combats.clear();
        }
        self.last_hour = Some(at);
        let previous_combats = self.combats.clone();
        let update = match &hour.countries {
            Countries::Incomplete => self.tags.update_projected(None, None),
            Countries::Same => Ok(false),
            Countries::Read(read) => self
                .tags
                .update_projected(Some(&read.keys), Some(&read.links)),
        };
        let links_known = update.is_ok();
        let identity_changed = matches!(update, Ok(true))
            && (self.tags.iter().count() != self.tag_identity.len()
                || self
                    .tags
                    .iter()
                    .any(|(raw, tag)| self.tag_identity.get(raw) != Some(tag)));
        let links_changed = links_known != self.links_known || identity_changed;
        self.result_links_dirty |= links_changed;
        self.projected.tags |= links_changed;
        if identity_changed {
            self.tag_identity = self
                .tags
                .iter()
                .map(|(raw, tag)| (*raw, tag.clone()))
                .collect();
        }
        self.links_known = links_known;

        let cpu = ::replay_frame::cpu::scope_phase("naval:combat_projection");
        let unchanged = contiguous
            && !links_changed
            && self
                .combat_hours
                .last()
                .is_some_and(|span| Some(span.last_hour) == previous_hour);
        match &hour.combats {
            Combats::Same if unchanged => {
                span(&mut self.combat_hours, at);
                // These samples are equal; only accepted coverage and Length advance.
                for &index in self.combats.values() {
                    self.episodes[index]
                        .runs
                        .last_mut()
                        .expect("complete active census")
                        .last = at;
                    self.dirty.insert(index);
                }
                if self.combat_incomplete != 0 {
                    self.refuse_rows(
                        "Active participant quantities or countries incomplete at read",
                        self.combat_incomplete,
                    );
                }
            }
            Combats::Same => {
                return Err(io::Error::other(
                    "A projected naval hour kept the combats its scan reads whole",
                ));
            }
            Combats::Unavailable => {
                self.refuse("Active combat census unavailable or incomplete");
                self.combats.clear();
                self.projected.incomplete_combats.clear();
            }
            Combats::Changed {
                all,
                samples,
                ended,
            } => {
                span(&mut self.combat_hours, at);
                if *all {
                    let present: HashSet<u64> = samples.iter().map(|sample| sample.id).collect();
                    self.combats.retain(|id, _| present.contains(id));
                    self.projected
                        .incomplete_combats
                        .retain(|id| present.contains(id));
                }
                for id in ended {
                    self.combats.remove(id);
                    self.projected.incomplete_combats.remove(id);
                }
                let mut sampled = HashSet::with_capacity(samples.len());
                for sample in samples {
                    sampled.insert(sample.id);
                    let index = self
                        .combats
                        .get(&sample.id)
                        .copied()
                        .filter(|index| self.episodes[*index].province == sample.province)
                        .unwrap_or_else(|| {
                            self.episode(sample.id, sample.province, Kind::Combat, at)
                        });
                    self.combats.insert(sample.id, index);
                    let snapshot = self.combat_sample(index, sample.sides.clone(), at);
                    if incomplete(&snapshot.sides) {
                        self.projected.incomplete_combats.insert(sample.id);
                    } else {
                        self.projected.incomplete_combats.remove(&sample.id);
                    }
                    self.push(index, at, snapshot);
                }
                // Every other present combat samples as the hour before.
                let kept: Vec<usize> = self
                    .combats
                    .iter()
                    .filter(|(id, _)| !sampled.contains(*id))
                    .map(|(_, &index)| index)
                    .collect();
                for index in kept {
                    self.episodes[index]
                        .runs
                        .last_mut()
                        .expect("complete active census")
                        .last = at;
                    self.dirty.insert(index);
                }
                self.combat_incomplete = self.projected.incomplete_combats.len();
                if self.combat_incomplete != 0 {
                    self.refuse_rows(
                        "Active participant quantities or countries incomplete at read",
                        self.combat_incomplete,
                    );
                }
            }
        }

        drop(cpu);
        let cpu = ::replay_frame::cpu::scope_phase("naval:settle_reconcile");
        let rows = match &hour.sources {
            Sources::Unavailable => None,
            Sources::Same => Some(std::mem::take(&mut self.projected.sources)),
            Sources::Rows(rows) => Some(rows.clone()),
        };
        let observed =
            self.observe_source_rows(rows.as_deref(), at, &previous_combats, previous_hour);
        if let Some(rows) = rows {
            self.projected.sources = rows;
        }
        self.projected.observed.extend(observed);
        drop(cpu);
        let _cpu = ::replay_frame::cpu::scope_phase("naval:results_projection");
        self.observe_results(hour, at, contiguous)
    }

    /// Writes held result `id`'s deferred extension, as the row path's held reads extended it.
    fn settle_result(&mut self, id: u64) {
        if let Some(through) = self.result_through
            && self.projected.present.contains(&id)
            && let Some(&index) = self.results.get(&id)
        {
            self.episodes[index]
                .runs
                .last_mut()
                .expect("deferred result read")
                .last = through;
            self.dirty.insert(index);
        }
    }

    fn observe_results(&mut self, hour: &Hour, at: i32, contiguous: bool) -> io::Result<()> {
        let links_changed = self.result_links_dirty;
        let (all, invalid, inputs, ended): (bool, Option<[usize; 4]>, &[ResultInput], &[u64]) =
            match &hour.results {
                Results::Unavailable => {
                    self.extend_results();
                    self.refuse("Retained result census unavailable or incomplete");
                    self.projected.views = false;
                    return Ok(());
                }
                Results::Same if contiguous && !links_changed && self.projected.views => {
                    span(&mut self.result_hours, at);
                    self.result_through = Some(at);
                    if self.result_incomplete != 0 {
                        self.refuse_rows(
                            "Result member classification or countries incomplete at read",
                            self.result_incomplete,
                        );
                    }
                    return Ok(());
                }
                Results::Same => (false, None, &[][..], &[][..]),
                Results::Changed {
                    all,
                    invalid,
                    inputs,
                    ended,
                } => (*all, Some(*invalid), inputs.as_slice(), ended.as_slice()),
            };
        if let Some(invalid) = invalid
            && invalid[0] != 0
        {
            self.extend_results();
            self.refuse_rows(
                "Invalid result selector or future date; read unavailable",
                invalid[0],
            );
            self.projected.views = false;
            return Ok(());
        }
        span(&mut self.result_hours, at);
        // A country change during a failed census must still invalidate the
        // next successful result read, even if its raw result rows are equal.
        self.result_links_dirty = false;
        let whole = all || !contiguous || !self.projected.views || self.projected.tags;
        self.projected.views = true;
        self.projected.tags = false;
        if let Some(invalid) = invalid {
            self.projected.refused = [invalid[1], invalid[2], invalid[3]];
        }
        for (count, reason) in self.projected.refused.into_iter().zip([
            "Invalid result member side; read unavailable",
            "Invalid result country list; read unavailable",
            "Invalid or duplicate result classification; read unavailable",
        ]) {
            if count != 0 {
                self.refuse_rows(reason, count);
            }
        }
        // Results the read no longer holds end at their last read.
        let mut gone: Vec<u64> = ended.to_vec();
        if all {
            let shipped: HashSet<u64> = inputs.iter().map(|input| input.row.id).collect();
            gone.extend(
                self.projected
                    .present
                    .iter()
                    .copied()
                    .filter(|id| !shipped.contains(id)),
            );
        }
        let mut regrouped = false;
        for id in gone {
            self.settle_result(id);
            regrouped |= self.projected.present.remove(&id);
            self.result_inputs.remove(&id);
            self.projected.incomplete_results.remove(&id);
        }
        // Only results whose inputs or source observation changed can sample anew.
        let mut touched: BTreeMap<u64, Option<&ResultInput>> = inputs
            .iter()
            .map(|input| (input.row.id, Some(input)))
            .collect();
        let observed = std::mem::take(&mut self.projected.observed);
        let held: Vec<u64> = if whole {
            self.projected.present.iter().copied().collect()
        } else {
            observed
                .into_iter()
                .filter(|id| self.projected.present.contains(id))
                .collect()
        };
        for id in held {
            touched.entry(id).or_insert(None);
        }
        for (id, shipped) in touched {
            self.settle_result(id);
            let input = match shipped {
                Some(input) => input.clone(),
                None => self
                    .result_inputs
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| io::Error::other("A held naval result has no inputs"))?,
            };
            let province = input.row.province;
            let before = self.results.get(&id).copied();
            let continuing = before.filter(|index| self.episodes[*index].province == province);
            let index = continuing.unwrap_or_else(|| {
                // A changed province is a new episode, never a permanent census refusal.
                let index = self.episode(id, province, Kind::Result, at);
                self.results.insert(id, index);
                index
            });
            regrouped |= self.projected.present.insert(id) || before != Some(index);
            let input_changed = self.result_inputs.get(&id) != Some(&input);
            if continuing.is_some() && !links_changed && !input_changed {
                self.dirty.insert(index);
                let last = self.episodes[index].runs.last_mut().expect("result read");
                if incomplete(&last.sample.sides) {
                    self.projected.incomplete_results.insert(id);
                } else {
                    self.projected.incomplete_results.remove(&id);
                }
                if last.last.checked_add(1) == Some(at) {
                    last.last = at;
                } else {
                    let sample = last.sample.clone();
                    self.push(index, at, sample);
                }
                continue;
            }
            let sample = self.result_sample(&input);
            if incomplete(&sample.sides) {
                self.projected.incomplete_results.insert(id);
            } else {
                self.projected.incomplete_results.remove(&id);
            }
            self.push(index, at, sample);
            if input_changed {
                self.result_inputs.insert(id, input);
            }
        }
        if regrouped {
            self.result_present = self
                .projected
                .present
                .iter()
                .map(|id| self.results[id])
                .collect();
        }
        self.result_incomplete = self.projected.incomplete_results.len();
        if self.result_incomplete != 0 {
            self.refuse_rows(
                "Result member classification or countries incomplete at read",
                self.result_incomplete,
            );
        }
        // Every held result reads through this hour until a read changes it.
        self.result_through = Some(at);
        Ok(())
    }
}

/// A branch's scan on the segment driver: each row window's pages are published once its last
/// hour is observed, as the windowed lane publishes each block it reads, and its naval losses
/// go into the family's series hour by hour.
pub struct Branch {
    scan: Box<Scan>,
    branch: u64,
    windows: Vec<BattleWindow>,
    /// The ordinal of the first window not yet published.
    next: usize,
    losses: naval_losses::prepared::emit::Emit,
    /// The branch's first hour, from which its series starts.
    first: i32,
    /// The last hour observed since the series was last cut.
    last: Option<i32>,
}

impl Branch {
    pub fn new(
        source: &Source,
        branch: u64,
        first: i32,
        content: &ships_sunk::PreparedContent,
    ) -> io::Result<Self> {
        let mut scan = Box::<Scan>::default();
        scan.configure(source, branch, content)?;
        Ok(Self {
            scan,
            branch,
            windows: windows(source.replay_ref(), branch),
            next: 0,
            losses: naval_losses::prepared::emit::Emit::default(),
            first,
            last: None,
        })
    }

    /// The hour the branch's series starts at.
    pub fn first(&self) -> i32 {
        self.first
    }

    /// Whether it observed an hour since its series was last cut.
    pub fn observed(&self) -> bool {
        self.last.is_some()
    }

    /// Observes `hour`, publishing every window that ends before it first.
    pub fn observe<W: Pages + ?Sized>(
        &mut self,
        writer: &mut W,
        hour: Hour,
        source: &Source,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let at = hour.hour;
        self.publish(source.replay_ref(), writer, |last| last < at, cancelled)?;
        self.scan.observe_projected(hour, source, cancelled)?;
        self.last = Some(at);
        self.publish(source.replay_ref(), writer, |last| last <= at, cancelled)
    }

    /// Appends the naval losses the last hour observed changed to the branch's series.
    pub fn append(&mut self, series: &mut Writer<'_>) -> io::Result<()> {
        let Some(at) = self.last else {
            return Ok(());
        };
        let (losses, _) = self.scan.losses();
        self.losses.hour(series, losses, at)
    }

    /// Cuts the branch's series through the last hour observed, its catalogue brought there.
    pub fn cut(&mut self, series: &mut Writer<'_>, cancelled: &AtomicBool) -> io::Result<()> {
        let Some(last) = self.last.take() else {
            return Ok(());
        };
        self.scan.settle(last);
        let (losses, scan) = self.scan.losses();
        self.losses.cut(series, losses, scan, last, cancelled)
    }

    /// What the branch goes on with after its last cut: its scan and the next window to
    /// publish, then its naval losses' open runs.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        if self.last.is_some() {
            return Err(io::Error::other("A naval branch is saved only after a cut"));
        }
        let json = serde_json::to_vec(&(&*self.scan, self.next))?;
        let packed = zstd::stream::encode_all(json.as_slice(), 3)?;
        out.extend_from_slice(&(packed.len() as u64).to_le_bytes());
        out.extend_from_slice(&packed);
        self.losses.save(out)
    }

    /// The branch [`Self::save`] saved, going on from the publication `directory` reads, whose
    /// series starts at `first`.
    pub fn resume(
        source: &Source,
        branch: u64,
        first: i32,
        content: &ships_sunk::PreparedContent,
        directory: &dashboard_series::family::Directory,
        saved: &[u8],
    ) -> io::Result<Self> {
        let invalid = || {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Saved naval branch is not valid",
            )
        };
        let length = saved
            .get(..8)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u64::from_le_bytes)
            .and_then(|length| usize::try_from(length).ok())
            .ok_or_else(invalid)?;
        let packed = saved.get(8..8 + length).ok_or_else(invalid)?;
        let (mut scan, next): (Box<Scan>, usize) =
            serde_json::from_slice(&zstd::stream::decode_all(packed)?)?;
        scan.configure(source, branch, content)?;
        let windows = windows(source.replay_ref(), branch);
        if next > windows.len() {
            return Err(invalid());
        }
        Ok(Self {
            scan,
            branch,
            windows,
            next,
            losses: naval_losses::prepared::emit::Emit::resume(directory, &saved[8 + length..])?,
            first,
            last: None,
        })
    }

    /// Publishes every window left once the branch's last hour is observed.
    pub fn finish<W: Pages + ?Sized>(
        &mut self,
        replay: &Replay,
        writer: &mut W,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.publish(replay, writer, |_| true, cancelled)
    }

    fn publish<W: Pages + ?Sized>(
        &mut self,
        replay: &Replay,
        writer: &mut W,
        due: impl Fn(i32) -> bool,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        while let Some(window) = self.windows.get(self.next)
            && due(window.last_hour)
        {
            self.scan
                .publish_configured(replay, writer, self.branch, self.next, cancelled)?;
            self.next += 1;
        }
        Ok(())
    }
}
