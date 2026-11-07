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

/// `reason` without the hour a failed table read names (`hour 123 Table …` becomes `Table …`):
/// refusals are counted by their words, once per observed hour, so a refusal that names its
/// hour would be a new reason every hour.
pub fn undated(reason: &str) -> std::borrow::Cow<'_, str> {
    let mut out = String::new();
    let mut rest = reason;
    let mut at = 0;
    let mut dated_any = false;
    while let Some(found) = rest[at..].find("hour ") {
        let start = at + found;
        let digits = &rest[start + 5..];
        let number = digits
            .char_indices()
            .take_while(|&(index, c)| c.is_ascii_digit() || (index == 0 && c == '-'))
            .count();
        let after = &digits[number..];
        let dated = (start == 0 || rest.as_bytes()[start - 1] == b' ')
            && number > 0
            && digits.as_bytes()[number - 1].is_ascii_digit()
            && after.starts_with(' ')
            && after[1..].starts_with(|c: char| c.is_ascii_uppercase());
        if dated {
            out.push_str(&rest[..start]);
            rest = &after[1..];
            at = 0;
            dated_any = true;
        } else {
            at = start + 5;
        }
    }
    if !dated_any {
        return std::borrow::Cow::Borrowed(reason);
    }
    out.push_str(rest);
    std::borrow::Cow::Owned(out)
}

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

/// [`Metadata`] borrowed, with the family's published `extra`.
#[derive(Serialize)]
struct MetadataView<'a, F, E> {
    fields: &'a BTreeMap<u64, F>,
    tags: &'a BTreeMap<u32, Vec<(i32, String)>>,
    refusals: &'a BTreeMap<String, u64>,
    extra: E,
}

/// Framing a catalogue change adds beyond its own text.
pub const EDIT_MARGIN: usize = 32;

#[derive(Serialize, Deserialize)]
#[serde(bound(
    serialize = "F: Serialize, X: Serialize",
    deserialize = "F: DeserializeOwned + Ord, X: DeserializeOwned"
))]
pub struct Keyed<F, X> {
    pub values: Accumulator,
    pub meta: Metadata<F, X>,
    last: Option<i32>,
    /// `meta.fields` inverted; rebuilt after a restore.
    #[serde(skip)]
    index: BTreeMap<F, u64>,
    /// Catalogue text changed since the last flush, from above.
    #[serde(skip)]
    changed: usize,
    /// Refusal reasons whose counts changed since the last flush.
    #[serde(skip)]
    refused: std::collections::BTreeSet<String>,
}

impl<F, X: Default> Default for Keyed<F, X> {
    fn default() -> Self {
        Self {
            values: Accumulator::default(),
            meta: Metadata {
                fields: BTreeMap::new(),
                tags: BTreeMap::new(),
                refusals: BTreeMap::new(),
                extra: X::default(),
            },
            last: None,
            index: BTreeMap::new(),
            changed: 0,
            refused: std::collections::BTreeSet::new(),
        }
    }
}

impl<F: Serialize + DeserializeOwned + Ord + Clone, X: Serialize + DeserializeOwned + Default>
    Keyed<F, X>
{
    /// Authenticate an accepted hour: whether it follows a gap (or is the
    /// first), so that every value is observed again.
    pub fn step(&mut self, hour: i32) -> io::Result<bool> {
        self.values.advance(hour)?;
        let gap = self.last.is_none_or(|last| last.checked_add(1) != Some(hour));
        self.last = Some(hour);
        Ok(gap)
    }

    pub fn key(&mut self, field: F) -> u64 {
        if let Some(key) = self.lookup(&field) {
            return key;
        }
        let name = serde_json::to_string(&field).expect("field keys serialize");
        let key = self.meta.fields.len() as u64;
        self.changed += 2 * name.len() + 20 + EDIT_MARGIN;
        self.meta.fields.insert(key, field.clone());
        self.index.insert(field, key);
        key
    }

    /// Each key by its published name: its field's text.
    pub fn keys(&self) -> io::Result<BTreeMap<String, u64>> {
        self.meta
            .fields
            .iter()
            .map(|(&key, field)| Ok((serde_json::to_string(field)?, key)))
            .collect()
    }

    /// The field's key, without giving one.
    pub fn lookup(&mut self, field: &F) -> Option<u64> {
        if self.index.len() != self.meta.fields.len() {
            self.index = self
                .meta
                .fields
                .iter()
                .map(|(&key, field)| (field.clone(), key))
                .collect();
        }
        self.index.get(field).copied()
    }

    pub fn keys_in(
        &mut self,
        range: impl std::ops::RangeBounds<F>,
    ) -> impl Iterator<Item = (&F, u64)> + '_ {
        if self.index.len() != self.meta.fields.len() {
            self.index = self
                .meta
                .fields
                .iter()
                .map(|(&key, field)| (field.clone(), key))
                .collect();
        }
        self.index.range(range).map(|(field, &key)| (field, key))
    }

    pub fn refuse(&mut self, reason: impl Into<String>) {
        self.refuse_times(reason.into(), 1);
    }

    pub fn refuse_times(&mut self, reason: impl AsRef<str>, times: u64) {
        if times == 0 {
            return;
        }
        let reason = undated(reason.as_ref());
        if !self.refused.contains(reason.as_ref()) {
            self.changed += 2 * reason.len() + 20 + EDIT_MARGIN;
            self.refused.insert(reason.as_ref().to_owned());
        }
        match self.meta.refusals.get_mut(reason.as_ref()) {
            Some(count) => *count += times,
            None => {
                self.meta.refusals.insert(reason.into_owned(), times);
            }
        }
    }

    /// Count `bytes` of the family's own catalogue facts as changed.
    pub fn note_change(&mut self, bytes: usize) {
        self.changed += bytes + EDIT_MARGIN;
    }

    /// Record the holder's tag at `hour` when it changed.
    pub fn tag(&mut self, tags: &Tags<'_>, slot: u32, hour: i32) {
        self.tag_text(slot, tags.slot_tag(slot), hour);
    }

    /// Record every identified holder's tag at `hour` where it changed.
    pub fn tag_all(&mut self, tags: &Tags<'_>, hour: i32) {
        for (slot, tag) in tags.tagged() {
            let runs = self.meta.tags.entry(slot).or_default();
            if runs.last().is_none_or(|(_, last)| last != tag) {
                self.changed += tag.len() + 16 + EDIT_MARGIN;
                runs.push((hour, tag.to_owned()));
            }
        }
    }

    pub fn tag_text(&mut self, slot: u32, tag: Option<String>, hour: i32) {
        if let Some(tag) = tag {
            let runs = self.meta.tags.entry(slot).or_default();
            if runs.last().is_none_or(|(_, last)| *last != tag) {
                self.changed += tag.len() + 16 + EDIT_MARGIN;
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

    /// What the next flush publishes: the values and the catalogue patch.
    pub fn pending(&self) -> usize {
        let catalogue = if self.changed == 0 {
            0
        } else {
            self.changed + CATALOGUE_MARGIN
        };
        self.values.bytes() + catalogue
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
        series::write_catalogue(writer, family, branch, &self.keys()?, &self.meta)
            .map_err(structural)?;
        self.published();
        Ok(())
    }

    pub fn flush_values(
        &mut self,
        writer: &mut store::Writer,
        family: &str,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.values
            .flush(writer, &values_section(family, branch), 0, cancelled)
    }

    /// The catalogue with `extra` as its facts; `base` notes the base range's length.
    #[allow(clippy::too_many_arguments)]
    pub fn publish_catalogue(
        &self,
        writer: &mut store::Writer,
        family: &str,
        branch: u64,
        catalogue: &mut series::CatalogueWriter,
        base: &mut Option<usize>,
        extra: &impl Serialize,
    ) -> io::Result<()> {
        let meta = MetadataView {
            fields: &self.meta.fields,
            tags: &self.meta.tags,
            refusals: &self.meta.refusals,
            extra,
        };
        catalogue
            .write(&mut BaseWatch { writer, base }, family, branch, &self.keys()?, &meta)
            .map_err(structural)
    }

    pub fn published(&mut self) {
        self.changed = 0;
        self.refused.clear();
    }

}

/// A malformed stored catalogue ends the run rather than refusing the family.
fn structural(error: io::Error) -> io::Error {
    if error.kind() == io::ErrorKind::InvalidData {
        io::Error::other(error)
    } else {
        error
    }
}

/// A catalogue patch's header and frame.
pub const CATALOGUE_MARGIN: usize = 256;

/// A writer noting the length of the base range it reads or writes.
struct BaseWatch<'a> {
    writer: &'a mut store::Writer,
    base: &'a mut Option<usize>,
}

impl dashboard_base::prepared::pages::Pages for BaseWatch<'_> {
    fn write_at(&mut self, section: &str, range: u64, data: &[u8]) -> io::Result<()> {
        dashboard_base::prepared::pages::Pages::write_at(self.writer, section, range, data)?;
        if range == 0 {
            *self.base = Some(data.len());
        }
        Ok(())
    }

    fn write(&mut self, section: &str, data: &[u8]) -> io::Result<u64> {
        dashboard_base::prepared::pages::Pages::write(self.writer, section, data)
    }

    fn read_at(&mut self, section: &str, range: u64) -> io::Result<Option<Vec<u8>>> {
        let bytes = dashboard_base::prepared::pages::Pages::read_at(self.writer, section, range)?;
        if range == 0
            && let Some(bytes) = &bytes
        {
            *self.base = Some(bytes.len());
        }
        Ok(bytes)
    }

    fn range_ids(
        &mut self,
        section: &str,
        count: u64,
    ) -> io::Result<Option<Vec<Option<store::RangeId>>>> {
        dashboard_base::prepared::pages::Pages::range_ids(self.writer, section, count)
    }
}

pub fn json_len(value: &impl Serialize) -> usize {
    struct Count(usize);
    impl io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count(0);
    serde_json::to_writer(&mut count, value).map_or(0, |()| count.0)
}

/// The most bytes zstd writes for `len` bytes.
pub fn compressed_bound(len: usize) -> usize {
    len + (len >> 8) + 64
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
        let runs = self.reader.runs(key, i32::MIN, self.through)?;
        Ok(self.clipped_text(runs, spans))
    }

    /// Each of `keys`' runs through the prepared prefix, unclipped, their blobs read together
    /// ([`Reader::runs_each`]): each key's, or the failure reading it alone gives.
    pub fn runs_each(&mut self, keys: &[u64]) -> io::Result<Vec<io::Result<Vec<Run>>>> {
        self.reader.runs_each(keys, i32::MIN, self.through)
    }

    /// A key's `runs` clipped to `spans` as exact decimal text, as [`Self::text`] gives them.
    pub fn clipped_text(&self, runs: Vec<Run>, spans: &[(i32, i32)]) -> Vec<EconomyRun> {
        clip(runs, spans, self.through)
            .into_iter()
            .map(|run| EconomyRun(run.first_hour, run.last_hour, run.value.to_string()))
            .collect()
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

pub fn merged(mut spans: Vec<(i32, i32)>) -> Vec<(i32, i32)> {
    spans.sort_unstable();
    let mut out: Vec<(i32, i32)> = Vec::with_capacity(spans.len());
    for (from, to) in spans {
        match out.last_mut() {
            Some(last) if from <= last.1.saturating_add(1) => last.1 = last.1.max(to),
            _ => out.push((from, to)),
        }
    }
    out
}

/// With a tag selection, a record answers only for hours when a chosen country was a party,
/// or may have been one: the hours of `unidentified`, while its other party had no identity.
pub fn selected_spans(
    chosen: Option<&BTreeMap<u32, Vec<(i32, i32)>>>,
    fixed: &[u32],
    parties: &[EconomyRun],
    unidentified: &[EconomyRun],
) -> Vec<(i32, i32)> {
    let Some(chosen) = chosen else {
        return vec![(i32::MIN, i32::MAX)];
    };
    let mut spans: Vec<(i32, i32)> = fixed
        .iter()
        .filter_map(|slot| chosen.get(slot))
        .flatten()
        .copied()
        .chain(unidentified.iter().map(|EconomyRun(first, last, _)| (*first, *last)))
        .collect();
    for EconomyRun(first, last, value) in parties {
        let Some(tagged) = value.parse::<u32>().ok().and_then(|slot| chosen.get(&slot)) else {
            continue;
        };
        for &(from, to) in tagged {
            let (from, to) = ((*first).max(from), (*last).min(to));
            if from <= to {
                spans.push((from, to));
            }
        }
    }
    merged(spans)
}

pub fn clip_text(runs: &[EconomyRun], spans: &[(i32, i32)]) -> Vec<EconomyRun> {
    let mut out = Vec::new();
    for EconomyRun(first, last, value) in runs {
        for &(from, to) in spans {
            let (from, to) = ((*first).max(from), (*last).min(to));
            if from <= to {
                out.push(EconomyRun(from, to, value.clone()));
            }
        }
    }
    out.sort_by_key(|run| run.0);
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

    /// Every slot with a readable tag, and that tag.
    pub fn tagged(&self) -> impl Iterator<Item = (u32, &'a str)> + '_ {
        self.slots.iter().filter_map(|(&slot, tag)| {
            std::str::from_utf8(tag)
                .ok()
                .filter(|tag| !tag.is_empty())
                .map(|tag| (slot, tag))
        })
    }

    pub fn slot_tag(&self, slot: u32) -> Option<String> {
        self.slots
            .get(&slot)
            .and_then(|tag| std::str::from_utf8(tag).ok())
            .filter(|tag| !tag.is_empty())
            .map(str::to_owned)
    }
}

/// Advance the variant catalogue over an hour; the versions it changed.
pub fn observe_variants(
    variants: &mut dashboard_base::products::equipment::VariantCatalogue,
    format: replay_data::format::FormatVersion,
    frame: &DecodedFrame,
    changes: Option<&dashboard_base::prepared::frame_changes::FrameChanges>,
) -> Vec<(u64, dashboard_base::products::equipment::VariantVersion)> {
    use replay_data::rows::TableId;
    let Some(changes) = changes else {
        variants.observe(format, frame);
        return Vec::new();
    };
    let unchanged = !dashboard_base::products::equipment::design_changed(frame)
        && !changes.slot_read[TableId::VariantDefinitions.index()]
        && dashboard_base::products::equipment::VariantCatalogue::TABLES
            .iter()
            .all(|table| !changes.rows_changed[table.index()]);
    if unchanged {
        variants.advance_changed(frame, changes);
        Vec::new()
    } else {
        variants.observe_changed(format, frame, changes)
    }
}

/// The branch id a request names.
pub fn branch_of(id: &str) -> io::Result<u64> {
    id.parse()
        .map_err(|_| invalid("Invalid economy history branch"))
}

#[cfg(test)]
mod undated_tests {
    use super::undated;

    #[test]
    fn a_read_names_its_table_without_its_hour() {
        assert_eq!(
            undated("market inventory unavailable: hour 100 CountryMarket is unavailable: x"),
            "market inventory unavailable: CountryMarket is unavailable: x"
        );
        assert_eq!(undated("hour -3 Countries is truncated (2 missing rows)"), "Countries is truncated (2 missing rows)");
        for kept in ["unavailable this hour: x", "hours 12 Countries", "hour 12 countries", "a war roster names a country with no identity"] {
            assert_eq!(undated(kept), kept);
        }
    }
}
