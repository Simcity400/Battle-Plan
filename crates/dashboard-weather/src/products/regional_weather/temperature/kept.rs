//! Temperature on a native recording's segments. Each segment's keeper derives every hour it
//! keeps from that hour's own rows and aggregates the region means ([`Deriving`]), so the
//! branch's observer only mirrors the means the keepers report and publishes those that
//! changed ([`Mirror`]). A derivation is a function of the hour's anchor words, pass and the
//! content alone, so a segment's keeper starting at its anchor gives what the walk's memo,
//! stepping from the hour before, gives.

use super::super::admission;
use super::{Bound, Layout, Observation, Resolved};
use dashboard_base::prepared::facts::Credit;
use dashboard_keyed::{Delta, Values, delta::Names, segment::Touch};
use replay_data::{rows::tables, temperature::Memo};
use replay_frame::frame::DecodedFrame;
use std::{io, sync::Arc};

const MISSING: &str = "region membership is empty or a member province temperature is unavailable";

fn invalid(what: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what)
}

fn reserved<T>(count: usize, owner: &'static str) -> io::Result<Vec<T>> {
    let mut values = Vec::new();
    values.try_reserve_exact(count).map_err(|error| {
        admission::pressure(owner, io::Error::new(io::ErrorKind::OutOfMemory, error))
    })?;
    Ok(values)
}

/// What a segment's keeper derived of one hour's temperatures.
pub(in super::super) enum Derived {
    /// No temperature is shown, for this reason.
    Refused(String),
    /// The means of the regions that changed since the keeper's last hour, by dense index,
    /// ascending; after none at all where `whole`. A region with no mean never has one.
    Means { whole: bool, changes: Vec<(u32, i64)> },
}

impl Derived {
    pub(in super::super) fn bytes(&self) -> usize {
        match self {
            Self::Refused(reason) => reason.capacity(),
            Self::Means { changes, .. } => changes.capacity() * size_of::<(u32, i64)>(),
        }
    }
}

/// A segment's derivation of the hours it keeps, bound to the content as the branch's
/// observer is.
#[derive(Default)]
pub(in super::super) struct Deriving {
    held: Option<Held>,
}

struct Held {
    content: Arc<Bound>,
    memo: Memo,
    /// Whether the last hour kept was derived: what the rows changed since certifies the next.
    derived: bool,
    /// The hour `sums` hold, when it was the last hour derived.
    summed: Option<i32>,
    sums: Vec<i128>,
    /// The means last reported, by dense index.
    means: Vec<Option<i64>>,
    /// No mean reported since the segment's start or the last refusal.
    fresh: bool,
    dirty: Vec<bool>,
    list: Vec<u32>,
    changes: Vec<(u32, i64)>,
    keys: Vec<u128>,
    _credit: Credit,
}

impl Deriving {
    /// The derivation the branch's observer binds alike; none where the content is
    /// unavailable, as the observer then has none.
    pub(in super::super) fn bind(content: &Arc<Resolved>) -> io::Result<Self> {
        let mut observation = Observation::default();
        observation.apply(content);
        let Some(content) = observation.content.take() else {
            return Ok(Self::default());
        };
        let count = content.layout().len();
        let credit = admission::admit(
            admission::add(
                admission::bytes::<(i128, Option<i64>, bool, u32, (u32, i64))>(count)?,
                // About one window of written slots.
                admission::bytes::<u128>(1024)?,
            )?,
            "segment region aggregates",
        )?;
        let mut held = Held {
            content,
            memo: Memo::default(),
            derived: false,
            summed: None,
            sums: reserved(count, "segment region aggregates")?,
            means: reserved(count, "segment region aggregates")?,
            fresh: true,
            dirty: reserved(count, "segment region aggregates")?,
            list: reserved(count, "segment region aggregates")?,
            changes: reserved(count, "segment region aggregates")?,
            keys: Vec::new(),
            _credit: credit,
        };
        held.sums.resize(count, 0);
        held.means.resize(count, None);
        held.dirty.resize(count, false);
        Ok(Self { held: Some(held) })
    }

    /// The hour `frame` gives, `touch` what it changed of the province rows; None without
    /// content.
    pub(in super::super) fn derive(
        &mut self,
        frame: &DecodedFrame,
        touch: &Touch,
    ) -> io::Result<Option<Derived>> {
        let Some(held) = &mut self.held else {
            return Ok(None);
        };
        held.memo.admit_changes(&held.content.pass)?;
        let derived = held.derivation(frame, touch);
        held.derived = derived.is_ok();
        match derived {
            Err(reason) => {
                held.summed = None;
                held.means.fill(None);
                held.fresh = true;
                Ok(Some(Derived::Refused(reason)))
            }
            Ok(()) => held.aggregate(frame.hour).map(Some),
        }
    }
}

impl Held {
    /// Derives the hour's temperatures, as the walk's observation does from its frame.
    fn derivation(&mut self, frame: &DecodedFrame, touch: &Touch) -> Result<(), String> {
        let rows = frame
            .complete::<tables::WeatherProvinces>()
            .map_err(|error| error.to_string())?;
        self.keys.clear();
        self.keys.extend(touch.written.iter().map(|&(key, _)| key));
        let changed = (self.derived && !touch.restated && touch.deleted.is_empty())
            .then_some(self.keys.as_slice());
        let content = &self.content;
        let states = self.memo.states_changed(
            frame.branch,
            frame.hour,
            rows,
            Ok(&content.planned()),
            changed,
        )?;
        if states.len() != content.pass.provinces.len() {
            return Err(format!(
                "content has {} weather province slots, the recording {}",
                content.pass.provinces.len(),
                states.len()
            ));
        }
        Ok(())
    }

    /// The region means of the hour just derived, as changes from those last reported.
    fn aggregate(&mut self, hour: i32) -> io::Result<Derived> {
        let layout = self.content.layout();
        let states = &self.memo.simulation().expect("a derived hour").states;
        let incremental = self.summed.and_then(|summed| summed.checked_add(1)) == Some(hour);
        self.changes.clear();
        match (incremental, self.memo.changes()) {
            (true, Some(changed)) => {
                for &(province, before, now) in changed {
                    for &index in layout.regions(province) {
                        let at = index as usize;
                        self.sums[at] += i128::from(now) - i128::from(before);
                        if !self.dirty[at] {
                            self.dirty[at] = true;
                            self.list.push(index);
                        }
                    }
                }
                self.list.sort_unstable();
                for &index in &self.list {
                    let at = index as usize;
                    self.dirty[at] = false;
                    let mean = layout.mean(at, self.sums[at]);
                    if mean != self.means[at] {
                        self.means[at] = mean;
                        if let Some(mean) = mean {
                            self.changes.push((index, mean));
                        }
                    }
                }
                self.list.clear();
            }
            _ => {
                for (at, members) in self.content.members.values().enumerate() {
                    self.sums[at] = members
                        .iter()
                        .filter_map(|&id| states.get(id as usize))
                        .map(|state| i128::from(state.temperature))
                        .sum();
                    let mean = layout.mean(at, self.sums[at]);
                    if mean != self.means[at] {
                        self.means[at] = mean;
                        if let Some(mean) = mean {
                            self.changes.push((at as u32, mean));
                        }
                    }
                }
            }
        }
        self.summed = Some(hour);
        let mut changes = reserved(self.changes.len(), "segment region means")?;
        changes.extend_from_slice(&self.changes);
        Ok(Derived::Means {
            whole: std::mem::take(&mut self.fresh),
            changes,
        })
    }
}

/// The region means a branch's keepers last reported and those its observer last published,
/// by dense index, with their names and the keys the scan numbered them with.
pub(super) struct Mirror {
    names: Vec<String>,
    keys: Vec<Option<u64>>,
    kept: Vec<Option<i64>>,
    published: Vec<Option<i64>>,
    dirty: Vec<bool>,
    list: Vec<u32>,
    _credit: Credit,
}

impl Mirror {
    fn new(layout: &Layout) -> io::Result<Self> {
        let count = layout.len();
        let per_region = admission::add(
            size_of::<(String, Option<u64>, Option<i64>, Option<i64>, bool, u32)>(),
            15,
        )?;
        let credit = admission::admit(
            count.checked_mul(per_region).ok_or_else(|| {
                admission::pressure("mirror size", io::ErrorKind::OutOfMemory.into())
            })?,
            "region temperature mirror",
        )?;
        let mut names = reserved(count, "region temperature mirror")?;
        for &id in &layout.ids {
            names.push(admission::name(id, "temp")?);
        }
        let mut mirror = Self {
            names,
            keys: reserved(count, "region temperature mirror")?,
            kept: reserved(count, "region temperature mirror")?,
            published: reserved(count, "region temperature mirror")?,
            dirty: reserved(count, "region temperature mirror")?,
            list: reserved(count, "region temperature mirror")?,
            _credit: credit,
        };
        mirror.keys.resize(count, None);
        mirror.kept.resize(count, None);
        mirror.published.resize(count, None);
        mirror.dirty.resize(count, false);
        Ok(mirror)
    }

    fn mark(&mut self, at: usize) {
        if !self.dirty[at] {
            self.dirty[at] = true;
            self.list.push(at as u32);
        }
    }

    /// Takes in what a keeper derived of an hour.
    fn take(&mut self, derived: &Derived) -> io::Result<()> {
        let (whole, changes) = match derived {
            Derived::Refused(_) => (true, &[][..]),
            Derived::Means { whole, changes } => (*whole, changes.as_slice()),
        };
        if whole {
            for at in 0..self.kept.len() {
                if self.kept[at].take().is_some() {
                    self.mark(at);
                }
            }
        }
        for &(index, mean) in changes {
            let at = index as usize;
            let held = self
                .kept
                .get_mut(at)
                .ok_or_else(|| invalid("regional-weather: a keeper's region is outside the content"))?;
            if *held != Some(mean) {
                *held = Some(mean);
                self.mark(at);
            }
        }
        Ok(())
    }

    /// Publishes each mean that differs from its publication, in region order.
    fn publish(&mut self, mut publish: impl FnMut(&str, &mut Option<u64>, Option<i128>)) {
        self.list.sort_unstable();
        for &index in &self.list {
            let at = index as usize;
            self.dirty[at] = false;
            if self.kept[at] != self.published[at] {
                self.published[at] = self.kept[at];
                publish(
                    &self.names[at],
                    &mut self.keys[at],
                    self.kept[at].map(i128::from),
                );
            }
        }
        self.list.clear();
    }

    pub(super) fn bytes(&self) -> usize {
        self.names.iter().map(String::capacity).sum::<usize>()
            + self.names.capacity() * size_of::<String>()
            + self.keys.capacity() * size_of::<Option<u64>>()
            + (self.kept.capacity() + self.published.capacity()) * size_of::<Option<i64>>()
            + self.dirty.capacity()
            + self.list.capacity() * size_of::<u32>()
    }
}

impl Observation {
    /// A keeper's hour the branch's emit observes none of: its means go on in the mirror.
    pub(in super::super) fn skip_kept(&mut self, derived: Option<&Derived>) -> io::Result<()> {
        let result = (|| {
            let (Some(content), Some(derived)) = (self.content.clone(), derived) else {
                return Ok(());
            };
            self.mirror(&content)?.take(derived)
        })();
        if result.is_err() {
            self.retire();
        }
        result
    }

    /// A keeper's hour observed whole: whether its temperatures are known, and the mean of
    /// each region that has one.
    pub(in super::super) fn observe_kept(
        &mut self,
        derived: Option<&Derived>,
        values: &mut Values,
    ) -> io::Result<bool> {
        let known = self.kept(derived, |_, _, _| {})?;
        if known && let Some(mirror) = &self.mirror {
            for at in 0..mirror.names.len() {
                if let Some(mean) = mirror.published[at] {
                    match mirror.keys[at] {
                        Some(key) => values.push_key(key, i128::from(mean)),
                        None => values.push(mirror.names[at].clone(), i128::from(mean)),
                    }
                }
            }
        }
        Ok(known)
    }

    /// A keeper's hour as the means it changed, by key where the scan numbered the name.
    pub(in super::super) fn observe_kept_changes(
        &mut self,
        derived: Option<&Derived>,
        names: &Names<'_>,
        delta: &mut Delta,
    ) -> io::Result<bool> {
        self.kept(derived, |name, key, value| {
            if key.is_none() {
                *key = names.key(name);
            }
            match (*key, value) {
                (Some(key), Some(value)) => delta.set_key(key, value),
                (Some(key), None) => delta.remove_key(key),
                (None, Some(value)) => delta.set(name.to_owned(), value),
                // A name the scan never numbered holds no value to end.
                (None, None) => {}
            }
        })
    }

    fn mirror(&mut self, content: &Bound) -> io::Result<&mut Mirror> {
        if self.mirror.is_none() {
            self.mirror = Some(Mirror::new(content.layout())?);
        }
        Ok(self.mirror.as_mut().unwrap())
    }

    fn kept(
        &mut self,
        derived: Option<&Derived>,
        publish: impl FnMut(&str, &mut Option<u64>, Option<i128>),
    ) -> io::Result<bool> {
        let result = self.kept_inner(derived, publish);
        if result.is_err() {
            self.retire();
        }
        result
    }

    fn kept_inner(
        &mut self,
        derived: Option<&Derived>,
        publish: impl FnMut(&str, &mut Option<u64>, Option<i128>),
    ) -> io::Result<bool> {
        let Some(content) = self.content.clone() else {
            if self.status.as_deref() == Some("available") {
                self.status = None;
            }
            return Ok(false);
        };
        let derived = derived.ok_or_else(|| {
            invalid("regional-weather: a segment's keeper holds no temperature content")
        })?;
        let mirror = self.mirror(&content)?;
        mirror.take(derived)?;
        mirror.publish(publish);
        let layout = content.layout();
        match derived {
            Derived::Refused(reason) => {
                self.certified = None;
                count(&mut self.refused, reason, 1);
                Ok(false)
            }
            Derived::Means { .. } => {
                if layout.missing > 0 {
                    count(&mut self.refused, MISSING, layout.missing);
                }
                Ok(layout.missing < layout.len() as u64)
            }
        }
    }
}

/// Adds `by` to `reason`'s count, spelling it only the first time.
fn count(refused: &mut std::collections::BTreeMap<String, u64>, reason: &str, by: u64) {
    match refused.get_mut(reason) {
        Some(count) => *count += by,
        None => {
            refused.insert(reason.to_owned(), by);
        }
    }
}
