//! Keyed state runs shared by the economy families: each family names its
//! values by a field (its holder and what it holds), observes an hour whole
//! when a watched table changed or after a gap, and publishes coalesced runs
//! through the shared prepared series and catalogue.
//!
//! A value starts at the first read showing it and ends at the first read
//! that does not. A gap ends every value at its last read: nothing is carried
//! across it. An unreadable table leaves its values unknown (no run) and
//! names its refusal; it never becomes zero or an empty list.
//!
//! Holders are native canonical country slots. Each keeps its tag runs, so a
//! query by tag reads only the hours the holder had the tag: a reused tag
//! never joins an unrelated holder.

use dashboard_base::{
    invalid,
    prepared::series::{self, Accumulator, Reader, Run},
};
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::BTreeMap,
    io,
    sync::atomic::{AtomicBool, Ordering},
};
use ts_rs::TS;

/// One observed hour's values, by key.
pub type Observed = BTreeMap<u64, i128>;

/// What a family's catalogue says about each key and holder.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Metadata<F, X> {
    /// Each key's field.
    pub fields: BTreeMap<u64, F>,
    /// Each holder slot's tag from each hour it was first read with it.
    pub tags: BTreeMap<u32, Vec<(i32, String)>>,
    /// Named reasons, each counted once per observed hour it held.
    pub refusals: BTreeMap<String, u64>,
    /// The family's own catalogue facts.
    pub extra: X,
}

#[derive(Serialize, Deserialize)]
#[serde(bound(
    serialize = "F: Serialize, X: Serialize",
    deserialize = "F: DeserializeOwned + Ord, X: DeserializeOwned"
))]
pub struct Keyed<F, X> {
    pub values: Accumulator,
    keys: BTreeMap<String, u64>,
    pub meta: Metadata<F, X>,
    last: Option<i32>,
}

impl<F, X: Default> Default for Keyed<F, X> {
    fn default() -> Self {
        Self {
            values: Accumulator::default(),
            keys: BTreeMap::new(),
            meta: Metadata {
                fields: BTreeMap::new(),
                tags: BTreeMap::new(),
                refusals: BTreeMap::new(),
                extra: X::default(),
            },
            last: None,
        }
    }
}

impl<F: Serialize + DeserializeOwned + Ord, X: Serialize + DeserializeOwned + Default> Keyed<F, X> {
    /// Authenticate an accepted hour: whether it follows a gap (or is the
    /// first), so that every value is observed again.
    pub fn step(&mut self, hour: i32) -> io::Result<bool> {
        self.values.advance(hour)?;
        let gap = self.last.is_none_or(|last| last.checked_add(1) != Some(hour));
        self.last = Some(hour);
        Ok(gap)
    }

    pub fn key(&mut self, field: F) -> u64 {
        let name = serde_json::to_string(&field).expect("field keys serialize");
        if let Some(&key) = self.keys.get(&name) {
            return key;
        }
        let key = self.keys.len() as u64;
        self.keys.insert(name, key);
        self.meta.fields.insert(key, field);
        key
    }

    pub fn refuse(&mut self, reason: impl Into<String>) {
        *self.meta.refusals.entry(reason.into()).or_default() += 1;
    }

    /// Record the holder's tag at `hour` when it changed.
    pub fn tag(&mut self, tags: &Tags<'_>, slot: u32, hour: i32) {
        if let Some(tag) = tags.slot_tag(slot) {
            let runs = self.meta.tags.entry(slot).or_default();
            if runs.last().is_none_or(|(_, last)| *last != tag) {
                runs.push((hour, tag));
            }
        }
    }

    /// The hour's whole state of the fields `scope` selects: every observed
    /// value, and the end of each held one it no longer shows.
    pub fn commit(&mut self, hour: i32, observed: &Observed, scope: impl Fn(&F) -> bool) {
        for (&key, &value) in observed {
            self.values
                .observe_if_changed(key, hour, Some(value), false, false);
        }
        let ended: Vec<u64> = self
            .values
            .active()
            .filter(|key| !observed.contains_key(key))
            .filter(|key| self.meta.fields.get(key).is_some_and(&scope))
            .collect();
        for key in ended {
            self.values.observe(key, hour, None, false);
        }
    }

    pub fn bytes(&self) -> usize {
        self.values.bytes()
    }

    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        family: &str,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.values
            .flush(writer, &values_section(family, branch), 0, cancelled)?;
        series::write_catalogue(writer, family, branch, &self.keys, &self.meta)
    }
}

pub fn values_section(family: &str, branch: u64) -> String {
    format!("{family}-values-{branch}")
}

/// A family's published catalogue and a reader of its values.
pub struct Published<'a, F, X> {
    pub meta: Metadata<F, X>,
    pub reader: Reader<'a>,
    pub through: i32,
    cancelled: &'a AtomicBool,
}

impl<'a, F: DeserializeOwned, X: DeserializeOwned> Published<'a, F, X> {
    /// None before the family's first publication on `branch`.
    pub fn open(
        store: &'a store::Store,
        key: &'a store::SourceKey,
        family: &str,
        branch: u64,
        through: i32,
        cancelled: &'a AtomicBool,
    ) -> io::Result<Option<Self>> {
        let Some((_, meta)) = series::read_catalogue::<Metadata<F, X>>(store, key, family, branch)?
        else {
            return Ok(None);
        };
        Ok(Some(Self {
            meta,
            reader: Reader::new(store, key, values_section(family, branch), cancelled)?,
            through,
            cancelled,
        }))
    }

    pub fn check(&self) -> io::Result<()> {
        if self.cancelled.load(Ordering::Acquire) {
            Err(invalid("Economy history request cancelled"))
        } else {
            Ok(())
        }
    }

    /// The key's runs through the prepared prefix, clipped to `spans`.
    pub fn runs(&mut self, key: u64, spans: &[(i32, i32)]) -> io::Result<Vec<Run>> {
        let runs = self.reader.runs(key, i32::MIN, self.through)?;
        Ok(clip(runs, spans, self.through))
    }

    /// The key's runs as exact decimal text.
    pub fn text(&mut self, key: u64, spans: &[(i32, i32)]) -> io::Result<Vec<EconomyRun>> {
        Ok(self
            .runs(key, spans)?
            .into_iter()
            .map(|run| EconomyRun(run.first_hour, run.last_hour, run.value.to_string()))
            .collect())
    }

    /// The value at `hour`, where a run holds one.
    pub fn at(&mut self, key: u64, hour: i32) -> io::Result<Option<i128>> {
        if hour > self.through {
            return Ok(None);
        }
        Ok(self.reader.value_at(key, hour)?.map(|run| run.value))
    }

    /// The holder's tag runs.
    pub fn tags(&self, slot: u32) -> Vec<HolderTag> {
        holder_tags(self.meta.tags.get(&slot).map(Vec::as_slice).unwrap_or_default())
    }

    /// The hours each holder had one of `tags`; every hour for an empty list.
    pub fn holder_spans(&self, tags: &[String]) -> BTreeMap<u32, Vec<(i32, i32)>> {
        self.meta
            .tags
            .iter()
            .filter_map(|(&slot, runs)| {
                let spans = if tags.is_empty() {
                    vec![(i32::MIN, i32::MAX)]
                } else {
                    tag_spans(runs, tags)
                };
                (!spans.is_empty()).then_some((slot, spans))
            })
            .collect()
    }

    /// The refusals as warning lines.
    pub fn warnings(&self, label: &str) -> Vec<String> {
        self.meta
            .refusals
            .iter()
            .map(|(reason, hours)| format!("{label}: {reason} ({hours} observed hours)"))
            .collect()
    }
}

/// Consecutive reads holding one value, as an exact decimal string:
/// `[firstHour, lastHour, value]`.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
pub struct EconomyRun(pub i32, pub i32, pub String);

/// A holder's tag from `hour` until its next entry.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct HolderTag {
    pub hour: i32,
    pub tag: String,
}

pub fn holder_tags(runs: &[(i32, String)]) -> Vec<HolderTag> {
    runs.iter()
        .map(|(hour, tag)| HolderTag {
            hour: *hour,
            tag: tag.clone(),
        })
        .collect()
}

/// The hours a holder's tag runs hold one of `tags`.
pub fn tag_spans(runs: &[(i32, String)], tags: &[String]) -> Vec<(i32, i32)> {
    runs.iter()
        .enumerate()
        .filter(|(_, (_, held))| tags.iter().any(|tag| tag == held))
        .map(|(index, (from, _))| {
            let to = runs
                .get(index + 1)
                .map_or(i32::MAX, |(next, _)| next.saturating_sub(1));
            (*from, to)
        })
        .collect()
}

/// The holder's tag at `hour`.
pub fn tag_at(runs: &[(i32, String)], hour: i32) -> Option<&str> {
    runs.iter()
        .rev()
        .find(|(from, _)| *from <= hour)
        .map(|(_, tag)| tag.as_str())
}

pub fn clip(runs: Vec<Run>, spans: &[(i32, i32)], through: i32) -> Vec<Run> {
    let mut out = Vec::new();
    for run in runs {
        for &(from, to) in spans {
            let first = run.first_hour.max(from);
            let last = run.last_hour.min(to).min(through);
            if first <= last {
                out.push(Run::new(first, last, run.value, run.break_before));
            }
        }
    }
    out.sort_by_key(|run| run.first_hour);
    out
}

/// Country identities at one read: native links to canonical slots, and
/// each slot's tag.
pub struct Tags<'a> {
    links: BTreeMap<u32, u32>,
    slots: BTreeMap<u32, &'a [u8]>,
}

impl<'a> Tags<'a> {
    pub fn new(frame: &'a DecodedFrame) -> io::Result<Self> {
        let countries = frame
            .complete::<tables::Countries>()
            .map_err(io::Error::other)?;
        let links = frame
            .complete::<tables::CountryLinks>()
            .map_err(io::Error::other)?;
        Ok(Self {
            links: links
                .iter()
                .map(|link| (link.id, link.canonical_slot))
                .collect(),
            slots: countries
                .iter()
                .map(|row| (row.canonical_slot, row.tag.as_slice()))
                .collect(),
        })
    }

    pub fn slot(&self, raw: u32) -> Option<u32> {
        self.links.get(&raw).copied()
    }

    pub fn slot_tag(&self, slot: u32) -> Option<String> {
        self.slots
            .get(&slot)
            .and_then(|tag| std::str::from_utf8(tag).ok())
            .filter(|tag| !tag.is_empty())
            .map(str::to_owned)
    }
}

/// Advance the variant catalogue over an accepted hour: observe it where a
/// naming table changed or was read, else carry its names.
pub fn observe_variants(
    variants: &mut dashboard_base::products::equipment::VariantCatalogue,
    format: replay_data::format::FormatVersion,
    frame: &DecodedFrame,
    changes: Option<&dashboard_base::prepared::frame_changes::FrameChanges>,
) {
    use replay_data::rows::TableId;
    let Some(changes) = changes else {
        variants.observe(format, frame);
        return;
    };
    let unchanged = !dashboard_base::products::equipment::design_changed(frame)
        && !changes.slot_read[TableId::VariantDefinitions.index()]
        && dashboard_base::products::equipment::VariantCatalogue::TABLES
            .iter()
            .all(|table| !changes.rows_changed[table.index()]);
    if unchanged {
        variants.advance(frame.hour);
    } else {
        let _ = variants.observe_changed(format, frame, changes);
    }
}

/// The branch id a request names.
pub fn branch_of(id: &str) -> io::Result<u64> {
    id.parse()
        .map_err(|_| invalid("Invalid economy history branch"))
}
