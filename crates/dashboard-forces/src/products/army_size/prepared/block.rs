//! Army size prepared block by block, as resource balance is: an extract from
//! the block's keyframe alone records the calls it makes after the block's
//! first hour; the fold observes that first hour on the carried state, makes
//! the recorded calls again under the carried keys (composition versions on
//! the carried counts) and carries every track forward. When the carried
//! state breaks an assumption of the fast fold, the fold observes the rest of
//! the block hour by hour.

use super::super::{Scan, Track};
use super::{Prepared, Publication, TABLES, publish_mix, scan_state};
use dashboard_base::prepared::{
    frame_changes::{FrameChangeTracker, FrameChanges},
    prepass::{Extract, Failure, Family, Folded},
    series::{self, Accumulator, Recording, ReplayTarget},
};
use replay::{
    container::{BlockIndex, Replay},
    format::FormatVersion,
    frame::DecodedFrame,
};
use std::{any::Any, collections::BTreeMap, io, sync::Arc};

pub static FAMILY: Family = Family {
    name: "army",
    extract: "army.extract",
    sequential: "army.sequential",
    fallback: "army.fallback",
    tables: TABLES,
    fields: None,
    start,
};

fn start(_: FormatVersion) -> Box<dyn Extract> {
    Box::new(BlockExtract {
        prepared: Box::default(),
        recording: Recording::default(),
        first: None,
    })
}

/// What a track held right after the block's first hour: how many runs,
/// compositions and identity segments, each at most one.
struct AfterFirst {
    runs: usize,
    mix: usize,
    segments: usize,
}

/// What the extract held right after the block's first hour.
struct First {
    frame: DecodedFrame,
    keys: usize,
    unreadable: usize,
    composition_unreadable: usize,
    invalid_table_reads: BTreeMap<String, usize>,
    tracks: BTreeMap<String, AfterFirst>,
}

struct BlockExtract {
    prepared: Box<Prepared>,
    recording: Recording,
    first: Option<First>,
}

struct BlockFacts {
    first: First,
    recording: Recording,
    names: Vec<String>,
    publications: Vec<Publication>,
    scan: Scan,
    last: i32,
}

impl Extract for BlockExtract {
    fn observe(
        &mut self,
        replay: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        if self.first.is_none() {
            self.prepared.observe(frame, replay, changes)?;
            let scan = &self.prepared.scan;
            self.first = Some(First {
                frame: frame.clone(),
                keys: self.prepared.tags.len(),
                unreadable: scan.unreadable,
                composition_unreadable: scan.composition_unreadable,
                invalid_table_reads: scan.invalid_table_reads.clone(),
                tracks: scan
                    .tracks
                    .iter()
                    .map(|(tag, track)| {
                        (
                            tag.clone(),
                            AfterFirst {
                                runs: track.runs.len(),
                                mix: track.mix.len(),
                                segments: track.segments.len(),
                            },
                        )
                    })
                    .collect(),
            });
            self.prepared.record_calls();
            return Ok(());
        }
        let prepared = &mut self.prepared;
        series::with_recording(&mut self.recording, || {
            prepared.observe(frame, replay, changes)
        })
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        let BlockExtract {
            prepared,
            recording,
            first,
        } = *self;
        let first = first.ok_or_else(|| io::Error::other("Army block has no first hour"))?;
        let prepared = *prepared;
        let names = series::catalogue_names(&prepared.tags)?;
        let mut scan = prepared.scan;
        scan.removed.clear();
        scan.observed.clear();
        Ok(Box::new(BlockFacts {
            first,
            recording,
            names,
            publications: prepared.recorded.unwrap_or_default(),
            scan,
            last: prepared.last,
        }))
    }
}

struct Target<'a> {
    prepared: &'a mut Prepared,
    keys: &'a [u64],
    publications: Vec<Option<Publication>>,
}

impl ReplayTarget for Target<'_> {
    fn series(&mut self, series: u8) -> &mut Accumulator {
        self.prepared.series(series)
    }
    fn key(&self, _: u8, key: u64) -> u64 {
        self.keys[key as usize]
    }
    fn family(&mut self, index: usize) -> io::Result<()> {
        let mut publication = self
            .publications
            .get_mut(index)
            .and_then(Option::take)
            .ok_or_else(|| io::Error::other("Army fold has no recorded composition"))?;
        publication.key = self.keys[publication.key as usize];
        publish_mix(
            &mut self.prepared.mix_ids,
            &mut self.prepared.parts,
            publication,
        )
    }
}

#[cfg(test)]
thread_local! {
    /// Fold every block fast, without checking its assumptions.
    static UNCHECKED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

impl Prepared {
    /// The accumulator recorded as `series`: the eight values, coverage, parts.
    fn series(&mut self, series: u8) -> &mut Accumulator {
        match series {
            8 => &mut self.coverage,
            9 => &mut self.parts,
            value => &mut self.values[usize::from(value)],
        }
    }

    fn record_calls(&mut self) {
        for series in 0..10 {
            self.series(series).record_as(series);
        }
        self.recorded = Some(Vec::new());
    }

    /// Fold one block's facts into the carried state, after the block's
    /// ordinary observation and before its publication.
    pub fn fold(
        &mut self,
        facts: Box<dyn Any + Send>,
        replay: &Replay,
        index: &BlockIndex,
        inject: &dyn Fn(i32, io::Result<()>) -> io::Result<()>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> Result<Folded, Failure> {
        let facts = facts.downcast::<BlockFacts>().map_err(|_| Failure {
            hour: index.first_hour,
            error: io::Error::other("Pre-pass facts of another family"),
        })?;
        let BlockFacts {
            first,
            recording,
            names,
            publications,
            scan,
            last,
        } = *facts;
        let hour = first.frame.hour;
        let changes = FrameChanges::alone(&first.frame, replay.format);
        inject(hour, self.observe(&first.frame, replay, &changes))
            .map_err(|error| Failure { hour, error })?;
        #[cfg(test)]
        let unchecked = UNCHECKED.with(std::cell::Cell::get);
        #[cfg(not(test))]
        let unchecked = false;
        if !unchecked && !self.continues(&first, &scan, &names) {
            self.observe_after_first(replay, index, hour, inject, check)?;
            self.forget_runtime();
            return Ok(Folded::Fallback);
        }
        let failed = |error: io::Error| Failure {
            hour: index.last_hour,
            error: io::Error::other(format!("Army block fold failed: {error}")),
        };
        let keys = series::number_names(&mut self.tags, &names);
        recording
            .replay(&mut Target {
                prepared: self,
                keys: &keys,
                publications: publications.into_iter().map(Some).collect(),
            })
            .map_err(failed)?;
        self.carry(&first, scan);
        self.last = last;
        self.forget_runtime();
        Ok(Folded::Fast)
    }

    /// Whether the carried state, after observing the block's first hour,
    /// continues as the extract did from it: every run, composition and
    /// segment the extract opened then continuing one observed with equal
    /// values, every track the extract met later closed before the block, and
    /// every key the first hour named carried.
    fn continues(&self, first: &First, block: &Scan, names: &[String]) -> bool {
        let hour = first.frame.hour;
        if !names[..first.keys.min(names.len())]
            .iter()
            .all(|name| self.tags.contains_key(name))
        {
            return false;
        }
        for (tag, extracted) in &block.tracks {
            let carried = self.scan.tracks.get(tag);
            match (first.tracks.get(tag), carried) {
                (Some(after), Some(carried)) => {
                    let opened = |count: usize, len: usize| count == 1 && len >= 1;
                    if opened(after.runs, extracted.runs.len())
                        && !carried.runs.last().is_some_and(|run| {
                            run.last == hour && run.totals == extracted.runs[0].totals
                        })
                    {
                        return false;
                    }
                    if opened(after.mix, extracted.mix.len())
                        && !carried.mix.last().is_some_and(|mix| {
                            mix.last == hour && mix.parts == extracted.mix[0].parts
                        })
                    {
                        return false;
                    }
                    if opened(after.segments, extracted.segments.len())
                        && !carried
                            .segments
                            .last()
                            .is_some_and(|segment| segment.last_hour == hour)
                    {
                        return false;
                    }
                    if after.runs > 1 || after.mix > 1 || after.segments > 1 {
                        return false;
                    }
                }
                (Some(_), None) => return false,
                (None, Some(carried)) => {
                    if carried.last.is_some_and(|(_, at)| at >= hour)
                        || carried
                            .segments
                            .last()
                            .is_some_and(|segment| segment.last_hour >= hour)
                    {
                        return false;
                    }
                }
                (None, None) => {}
            }
        }
        true
    }

    /// Carry every track forward from the extract's end of block: the run,
    /// composition and segment the extract opened in the first hour extend
    /// the carried ones observed then, later ones follow them, and counts add
    /// to the carried counts.
    fn carry(&mut self, first: &First, block: Scan) {
        let hour = first.frame.hour;
        let Scan {
            tracks,
            divisions,
            strength,
            manpower,
            inventory_fallback,
            unreadable,
            battalions,
            composition_unreadable,
            invalid_table_reads,
            frontier,
            live,
            ..
        } = block;
        for (tag, extracted) in tracks {
            let after = first.tracks.get(&tag);
            let carried = self.scan.tracks.entry(tag).or_default();
            carry_track(carried, extracted, after, hour);
        }
        let scan = &mut self.scan;
        scan.divisions |= divisions;
        scan.strength |= strength;
        scan.manpower |= manpower;
        scan.inventory_fallback |= inventory_fallback;
        scan.battalions |= battalions;
        scan.unreadable += unreadable - first.unreadable;
        scan.composition_unreadable += composition_unreadable - first.composition_unreadable;
        for (read, count) in invalid_table_reads {
            let before = first.invalid_table_reads.get(&read).copied().unwrap_or(0);
            *scan.invalid_table_reads.entry(read).or_default() += count - before;
        }
        scan.frontier = frontier;
        scan.live = live;
    }

    /// Drop what the observer rebuilds from its next frame, as a restored
    /// observer has none of it.
    fn forget_runtime(&mut self) {
        self.state = scan_state();
        self.scan.removed.clear();
        self.scan.observed.clear();
        for track in self.scan.tracks.values_mut() {
            track.mix_version = None;
        }
    }

    /// Observe the block again hour by hour after its first hour, which the
    /// fold already observed, with the block's own change facts.
    fn observe_after_first(
        &mut self,
        replay: &Replay,
        index: &BlockIndex,
        first: i32,
        inject: &dyn Fn(i32, io::Result<()>) -> io::Result<()>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> Result<(), Failure> {
        let mut tracker = FrameChangeTracker::new();
        let mut failure = None;
        let read = replay.read_block_alone(
            index,
            TABLES,
            |raw, frame| {
                let changes = tracker.observe_updates(frame, Arc::new(raw), replay.format);
                if frame.hour == first {
                    return Ok(());
                }
                if let Err(error) = inject(frame.hour, self.observe(frame, replay, &changes)) {
                    failure = Some(Failure {
                        hour: frame.hour,
                        error,
                    });
                    return Err(io::Error::other("army observation stopped"));
                }
                Ok(())
            },
            check,
        );
        if let Some(failure) = failure {
            return Err(failure);
        }
        read.map_err(|error| Failure { hour: first, error })
    }
}

fn carry_track(carried: &mut Track, extracted: Track, after: Option<&AfterFirst>, hour: i32) {
    let Track {
        mut runs,
        last,
        mut segments,
        last_identity,
        mut mix,
        ..
    } = extracted;
    let opened = |count: Option<usize>| count == Some(1);
    if opened(after.map(|after| after.runs))
        && runs.first().is_some_and(|run| run.first == hour)
        && let Some(open) = carried.runs.last_mut()
    {
        open.last = runs[0].last;
        runs.remove(0);
    }
    carried.runs.extend(runs);
    if opened(after.map(|after| after.mix))
        && mix.first().is_some_and(|run| run.first == hour)
        && let Some(open) = carried.mix.last_mut()
    {
        open.last = mix[0].last;
        mix.remove(0);
    }
    carried.mix.extend(mix);
    let observed = !segments.is_empty();
    if opened(after.map(|after| after.segments))
        && segments
            .first()
            .is_some_and(|segment| segment.first_hour == hour)
        && let Some(open) = carried.segments.last_mut()
    {
        open.last_hour = segments[0].last_hour;
        segments.remove(0);
    }
    carried.segments.extend(segments);
    if last.is_some() {
        carried.last = last;
    }
    if observed {
        carried.last_identity = last_identity;
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_fixtures;
#[cfg(test)]
mod tests;
