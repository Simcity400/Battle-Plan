//! Country styles prepared on their own as a keyed family ([`dashboard_keyed`]): each hour's
//! style inputs ([`StyleInputs`]) kept as runs of numbered names, so `country-styles` and
//! `country-colours` answer every hour preparation holds from those runs ([`PreparedStyles`]),
//! exactly as a seek of the hour reads it. A tag's run holds its first country's raw link id and
//! canonical slot, a raw link id's its map colour, and a canonical slot's its naming operands,
//! interned by value so a run holds their index.

use crate::{invalid, metadata::check_cancel};
use dashboard_base::{
    Source,
    prepared::{coverage, prepass::Family, series},
    products::{
        ContentOptions,
        country_styles::{CountryFactRuns, CountryFactsReply, HourStyles, StyleInputs, StyleRuns},
    },
};
use dashboard_keyed::{KNOWN, Observer, Scan, Values};
use installed_content_country_names::country_names::{Operands, TABLES as STYLE_TABLES};
use installed_content_localisation::localisation::Localisation;
use replay::container::TableFields;
use replay_data::{format::FormatVersion, rows::TableId};
use replay_frame::frame::{DecodedFrame, TableView};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap},
    hash::Hash,
    io,
    sync::{Arc, PoisonError, atomic::AtomicBool},
};

/// The family's store name; its coverage gates both style products.
pub const NAME: &str = "country-styles";

pub static FAMILY: Family = dashboard_keyed::block::family::<StylesObserver>(
    NAME,
    "country-styles.extract",
    "country-styles.sequential",
    "country-styles.fallback",
);

/// The name holding which of slot zero's names its reserved row has built, -1 for none.
const BUILT: &str = "built";

/// Requested hours read and visited at a time, so a reply's budget stops the read within one.
/// Tests take a few, so their short fixtures cross windows.
#[cfg(not(test))]
const WINDOW: usize = 256;
#[cfg(test)]
const WINDOW: usize = 4;

/// The tables whose incompleteness a style names with its hour: an hour any of them is not
/// complete is left to a seek, so no kept text names another hour.
const WHOLE: [TableId; 4] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::ReservedCountryNaming,
];

/// One country's naming operands as their runs keep them ([`Operands::parts`]).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct Parts {
    overlord: Option<u32>,
    texts: [String; 6],
}

impl Parts {
    fn of(operands: &Operands) -> Self {
        let (overlord, texts) = operands.parts();
        Self {
            overlord,
            texts: texts.map(str::to_owned),
        }
    }

    fn operands(&self) -> Operands {
        Operands::from_parts(self.overlord, self.texts.clone())
    }
}

/// Values numbered in first-seen order, so a run holds a value's index.
#[derive(Serialize, Deserialize)]
#[serde(bound(
    serialize = "T: Serialize",
    deserialize = "T: Deserialize<'de> + Eq + Hash"
))]
struct Interned<T: Eq + Hash> {
    values: Vec<T>,
    #[serde(skip)]
    index: HashMap<T, i128>,
}

impl<T: Eq + Hash> Default for Interned<T> {
    fn default() -> Self {
        Self {
            values: Vec::new(),
            index: HashMap::new(),
        }
    }
}

impl<T: Clone + Eq + Hash> Interned<T> {
    fn number(&mut self, value: T) -> i128 {
        if self.index.len() != self.values.len() {
            self.index = (0..)
                .zip(&self.values)
                .map(|(at, value)| (value.clone(), at))
                .collect();
        }
        if let Some(&at) = self.index.get(&value) {
            return at;
        }
        let at = self.values.len() as i128;
        self.values.push(value.clone());
        self.index.insert(value, at);
        at
    }
}

/// Each hour's style inputs as numbered names; it interns the naming operands.
#[derive(Default, Serialize, Deserialize)]
pub struct StylesObserver {
    operands: Interned<Result<Parts, String>>,
}

fn country(id: u32, slot: u32) -> i128 {
    (i128::from(id) << 32) | i128::from(slot)
}

fn colour([red, green, blue]: [u8; 3]) -> i128 {
    (i128::from(red) << 16) | (i128::from(green) << 8) | i128::from(blue)
}

impl Observer for StylesObserver {
    const NAME: &'static str = NAME;
    const TABLES: &'static [TableId] = &STYLE_TABLES;
    const READ: TableId = TableId::Countries;

    fn family() -> &'static Family {
        &FAMILY
    }

    /// The fields the styles and their naming operands read.
    fn fields() -> io::Result<Vec<TableFields>> {
        Ok(vec![
            TableFields::named(TableId::Countries, &["canonical_slot", "tag"])?,
            TableFields::all(TableId::CountryLinks),
            TableFields::named(
                TableId::CountryAppearance,
                &[
                    "red",
                    "green",
                    "blue",
                    "cosmetic_tag",
                    "name_tag",
                    "leader_ideology",
                    "overlord",
                ],
            )?,
            TableFields::named(TableId::Politics, &["ruling_party"])?,
            TableFields::named(TableId::PoliticalParties, &["token"])?,
            TableFields::named(TableId::Autonomy, &["state"])?,
            TableFields::named(
                TableId::ReservedCountryNaming,
                &[
                    "tag",
                    "name_tag",
                    "cosmetic_tag",
                    "leader_ideology",
                    "group",
                    "overlord",
                    "autonomy",
                    "built",
                ],
            )?,
        ])
    }

    fn observe(
        &mut self,
        _: FormatVersion,
        frame: &DecodedFrame,
        values: &mut Values,
    ) -> io::Result<bool> {
        let whole = WHOLE
            .iter()
            .all(|table| matches!(&*frame.tables[table.index()], TableView::Complete(_)));
        let inputs = match whole.then(|| StyleInputs::read(frame, true)) {
            Some(Ok(inputs)) => inputs,
            _ => return Ok(false),
        };
        values.push(
            BUILT.to_owned(),
            inputs.reserved_built.map_or(-1, i128::from),
        );
        for (tag, (id, slot)) in inputs.tags {
            values.push(format!("t:{tag}"), country(id, slot));
        }
        for (id, color) in inputs.colors {
            values.push(format!("c:{id}"), colour(color));
        }
        for (slot, operands) in inputs.operands {
            let parts = operands.as_ref().map(Parts::of).map_err(String::clone);
            values.push(format!("o:{slot}"), self.operands.number(parts));
        }
        Ok(true)
    }
}

/// Preparation's style inputs, which the products read through [`StyleRuns`].
pub struct PreparedStyles;

pub static PREPARED_STYLES: PreparedStyles = PreparedStyles;

/// A branch's latest catalogue: each name's key by what it names, and the interned operands.
struct Published {
    ranges: Vec<[u64; 3]>,
    built: Option<u64>,
    tags: HashMap<String, u64>,
    colours: HashMap<u32, u64>,
    slots: HashMap<u32, u64>,
    operands: Vec<Result<Parts, String>>,
}

impl Published {
    fn read(scan: Scan<StylesObserver>, ranges: Vec<[u64; 3]>) -> Self {
        let mut published = Self {
            ranges,
            built: None,
            tags: HashMap::new(),
            colours: HashMap::new(),
            slots: HashMap::new(),
            operands: Vec::new(),
        };
        for (name, key) in scan.keyed() {
            if name == BUILT {
                published.built = Some(key);
            } else if let Some(tag) = name.strip_prefix("t:") {
                published.tags.insert(tag.to_owned(), key);
            } else if let Some(id) = name.strip_prefix("c:").and_then(|id| id.parse().ok()) {
                published.colours.insert(id, key);
            } else if let Some(slot) = name.strip_prefix("o:").and_then(|slot| slot.parse().ok()) {
                published.slots.insert(slot, key);
            }
        }
        published.operands = scan.observer.operands.values;
        published
    }
}

/// Each branch's latest catalogue a request read, kept for the source's later requests.
#[derive(Default)]
struct Catalogues(HashMap<u64, Arc<Published>>);

/// `branch`'s latest catalogue, read again once any of its ranges is published anew.
fn published(
    source: &Source,
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
) -> io::Result<Option<Arc<Published>>> {
    let Some(ranges) = series::catalogue_ranges(store, key, NAME, branch)? else {
        return Ok(None);
    };
    let ranges: Vec<[u64; 3]> = ranges.iter().map(store::RangeId::cache_key).collect();
    let state = source.state::<Catalogues>();
    let held = state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .0
        .get(&branch)
        .filter(|held| held.ranges == ranges)
        .cloned();
    if let Some(held) = held {
        return Ok(Some(held));
    }
    let Some((_, scan)) = series::read_catalogue::<Scan<StylesObserver>>(store, key, NAME, branch)?
    else {
        return Ok(None);
    };
    let read = Arc::new(Published::read(scan, ranges));
    state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .0
        .insert(branch, Arc::clone(&read));
    Ok(Some(read))
}

/// Each key's value at each requested hour in turn, from its changes
/// ([`series::Reader::changes_at`]).
struct Sweep {
    changes: Vec<Vec<(usize, Option<i128>)>>,
    next: Vec<usize>,
    current: Vec<Option<i128>>,
}

impl Sweep {
    fn new(changes: Vec<Vec<(usize, Option<i128>)>>) -> Self {
        Self {
            next: vec![0; changes.len()],
            current: vec![None; changes.len()],
            changes,
        }
    }

    /// Takes every key to its value at the requested hour `at`.
    fn advance(&mut self, at: usize) {
        for (key, changes) in self.changes.iter().enumerate() {
            while let Some(&(_, value)) =
                changes.get(self.next[key]).filter(|(hour, _)| *hour <= at)
            {
                self.current[key] = value;
                self.next[key] += 1;
            }
        }
    }
}

impl StyleRuns for PreparedStyles {
    fn inputs(
        &self,
        source: &Source,
        content: Option<&ContentOptions>,
        branch: u64,
        hours: &[i32],
        tags: &[String],
        names: bool,
        cancelled: &AtomicBool,
        visit: &mut dyn FnMut(i32, StyleInputs) -> io::Result<()>,
    ) -> io::Result<bool> {
        let (Some(&first), Some(&last)) = (hours.first(), hours.last()) else {
            return Ok(true);
        };
        let Some(coverage) = coverage::product_coverage(source, content, branch, NAME, cancelled)?
        else {
            return Ok(false);
        };
        if first < coverage.first_hour || (!coverage.complete && last > coverage.last_hour) {
            return Ok(false);
        }
        let (store, key) = dashboard_base::prepared::files(source, content)?;
        let _scope = store.scope(&key)?;
        let Some(published) = published(source, &store, &key, branch)? else {
            return Ok(false);
        };
        let Some(built) = published.built else {
            return Ok(false);
        };
        let mut reader = series::Reader::new(
            &store,
            &key,
            dashboard_keyed::series_section(NAME, branch),
            cancelled,
        )?;
        // Every hour preparation read, or none answered here: an hour it did not is a seek's.
        let known = reader.runs(KNOWN, first, last)?;
        let held = |hour: i32| {
            let at = known.partition_point(|run| run.last_hour < hour);
            known.get(at).is_some_and(|run| run.first_hour <= hour)
        };
        if !hours.iter().all(|&hour| held(hour)) {
            return Ok(false);
        }
        let named: Vec<(&String, u64)> = tags
            .iter()
            .filter_map(|tag| published.tags.get(tag).map(|&key| (tag, key)))
            .collect();
        let mut keys = vec![built];
        keys.extend(named.iter().map(|&(_, key)| key));
        // Each window starts from no value: its first hour's are among its changes.
        for window in hours.chunks(WINDOW) {
            let countries = reader.changes_at(&keys, window)?;
            // Every raw id the requested tags hold in the window, each with its colour key.
            let ids: BTreeSet<u32> = countries[1..]
                .iter()
                .flatten()
                .filter_map(|&(_, value)| value.map(|value| (value >> 32) as u32))
                .collect();
            let coloured: Vec<(u32, u64)> = ids
                .into_iter()
                .filter_map(|id| published.colours.get(&id).map(|&key| (id, key)))
                .collect();
            let colour_at: HashMap<u32, usize> = coloured
                .iter()
                .enumerate()
                .map(|(at, &(id, _))| (id, at))
                .collect();
            let keyed: Vec<u64> = coloured.iter().map(|&(_, key)| key).collect();
            let mut countries = Sweep::new(countries);
            let mut colours = Sweep::new(reader.changes_at(&keyed, window)?);
            for (at, &hour) in window.iter().enumerate() {
                check_cancel(cancelled)?;
                countries.advance(at);
                colours.advance(at);
                let mut inputs = StyleInputs {
                    reserved_built: countries.current[0]
                        .filter(|&built| built >= 0)
                        .map(|built| built as u8),
                    ..StyleInputs::default()
                };
                for (&(tag, _), value) in named.iter().zip(&countries.current[1..]) {
                    let Some(value) = *value else { continue };
                    let id = (value >> 32) as u32;
                    inputs.tags.insert(tag.clone(), (id, value as u32));
                    if let Some(value) = colour_at.get(&id).and_then(|&at| colours.current[at]) {
                        inputs
                            .colors
                            .insert(id, [(value >> 16) as u8, (value >> 8) as u8, value as u8]);
                    }
                }
                if names {
                    operands(&published, &mut reader, hour, &mut inputs)?;
                }
                visit(hour, inputs)?;
            }
        }
        Ok(true)
    }
}

/// Numbers texts by first use, as a reply's dictionary.
#[derive(Default)]
struct Dictionary {
    numbered: HashMap<String, i32>,
    texts: Vec<String>,
}

impl Dictionary {
    fn number(&mut self, text: &str) -> i32 {
        if let Some(&number) = self.numbered.get(text) {
            return number;
        }
        let number = self.texts.len() as i32;
        self.numbered.insert(text.to_owned(), number);
        self.texts.push(text.to_owned());
        number
    }
}

/// Every tag's style at every hour `branch`'s runs hold from `first` through `last`, named
/// with `localisation` ([`CountryFactsReply`]): exactly what [`PreparedStyles`] gives each
/// hour, kept as runs. Only the tags an hour's changes reach are styled again: a tag's own
/// link, its link's colour, any naming operands its name reaches, or the reserved names
/// built. None before preparation holds the family. Read through the prepared query's `store`.
pub(crate) fn country_facts(
    source: &Source,
    (store, key): (&store::Store, &store::SourceKey),
    branch: u64,
    [first, last]: [i32; 2],
    localisation: Option<&Localisation>,
    cancelled: &AtomicBool,
) -> io::Result<Option<CountryFactsReply>> {
    let _scope = store.scope(key)?;
    let Some(published) = published(source, store, key, branch)? else {
        return Ok(None);
    };
    let Some(built) = published.built else {
        return Ok(None);
    };
    let mut reply = CountryFactsReply {
        branch_id: branch.to_string(),
        ..CountryFactsReply::default()
    };
    if last < first {
        return Ok(Some(reply));
    }
    let mut reader = series::Reader::new(
        store,
        key,
        dashboard_keyed::series_section(NAME, branch),
        cancelled,
    )?;
    for run in reader.runs(KNOWN, first, last)? {
        match reply.known.last_mut() {
            Some(span) if span[1].checked_add(1) == Some(run.first_hour) => span[1] = run.last_hour,
            _ => reply.known.push([run.first_hour, run.last_hour]),
        }
    }
    let mut tags: Vec<(&String, u64)> = published.tags.iter().map(|(tag, &key)| (tag, key)).collect();
    tags.sort_unstable();
    let colours: Vec<(u32, u64)> = published.colours.iter().map(|(&id, &key)| (id, key)).collect();
    let slots: Vec<(u32, u64)> = published.slots.iter().map(|(&slot, &key)| (slot, key)).collect();
    // One order for every key: the reserved names built, then tags, colours and slots.
    let keys: Vec<u64> = std::iter::once(built)
        .chain(tags.iter().map(|&(_, key)| key))
        .chain(colours.iter().map(|&(_, key)| key))
        .chain(slots.iter().map(|&(_, key)| key))
        .collect();
    let runs = reader
        .runs_each(&keys, first, last)?
        .into_iter()
        .collect::<io::Result<Vec<_>>>()?;
    // Every key's value changes in hour order: a run's start, and its end the hour after its
    // last. An end sorts before a start at one hour, so a run following another keeps its value.
    let mut events: Vec<(i32, bool, usize, i128)> = Vec::new();
    for (at, runs) in runs.iter().enumerate() {
        for run in runs {
            events.push((run.first_hour, true, at, run.value));
            if let Some(end) = run.last_hour.checked_add(1) {
                events.push((end, false, at, 0));
            }
        }
    }
    events.sort_unstable_by_key(|&(hour, start, at, _)| (hour, start, at));
    let held = |hour: i32| {
        let at = reply.known.partition_point(|span| span[1] < hour);
        reply.known.get(at).is_some_and(|span| span[0] <= hour)
    };
    // Tags are styled at each known span's first hour and at every known hour a value changes;
    // changes in a gap are styled at the next span's first hour.
    let mut styled: BTreeSet<i32> = reply.known.iter().map(|span| span[0]).collect();
    styled.extend(events.iter().map(|&(hour, ..)| hour).filter(|&hour| held(hour)));
    let (tag_at, colour_at, slot_at) = (1, 1 + tags.len(), 1 + tags.len() + colours.len());
    let slot_of: HashMap<u32, usize> = slots
        .iter()
        .enumerate()
        .map(|(at, &(slot, _))| (slot, slot_at + at))
        .collect();
    let colour_of: HashMap<u32, usize> = colours
        .iter()
        .enumerate()
        .map(|(at, &(id, _))| (id, colour_at + at))
        .collect();
    // The overlord a slot's operands name, read without copying them.
    let overlord = |values: &[Option<i128>], slot: u32| -> Option<u32> {
        let value = slot_of.get(&slot).and_then(|&at| values[at])?;
        published
            .operands
            .get(usize::try_from(value).ok()?)?
            .as_ref()
            .ok()?
            .overlord
    };
    let operands_at = |values: &[Option<i128>], slot: u32| -> io::Result<Option<Result<Operands, String>>> {
        let Some(value) = slot_of.get(&slot).and_then(|&at| values[at]) else {
            return Ok(None);
        };
        usize::try_from(value)
            .ok()
            .and_then(|at| published.operands.get(at))
            .ok_or_else(|| invalid("Prepared country naming operands are not catalogued"))
            .map(|parts| Some(parts.as_ref().map(Parts::operands).map_err(String::clone)))
    };
    let mut values: Vec<Option<i128>> = vec![None; keys.len()];
    let mut changed: Vec<usize> = Vec::new();
    let mut marked = vec![false; keys.len()];
    let mut next = 0;
    // Each tag's link and the slots its name reaches, and who depends on what.
    let mut links: Vec<Option<(u32, u32)>> = vec![None; tags.len()];
    let mut chains: Vec<Vec<u32>> = vec![Vec::new(); tags.len()];
    let mut holding: HashMap<u32, BTreeSet<usize>> = HashMap::new();
    let mut reaching: HashMap<u32, BTreeSet<usize>> = HashMap::new();
    let mut names = Dictionary::default();
    let mut reasons = Dictionary::default();
    let mut shown: Vec<Option<[i32; 3]>> = vec![None; tags.len()];
    let mut countries: Vec<Vec<[i32; 4]>> = vec![Vec::new(); tags.len()];
    let mut first_hour = true;
    for hour in styled {
        check_cancel(cancelled)?;
        while let Some(&(_, start, at, value)) = events.get(next).filter(|event| event.0 <= hour) {
            next += 1;
            let value = start.then_some(value);
            if values[at] != value {
                values[at] = value;
                if !std::mem::replace(&mut marked[at], true) {
                    changed.push(at);
                }
            }
        }
        // The tags this hour's changes reach: all of them when the reserved names built change.
        let mut reached = BTreeSet::new();
        if first_hour || marked[0] {
            reached.extend(0..tags.len());
        } else {
            for &at in &changed {
                let dependents = if at < colour_at {
                    reached.insert(at - tag_at);
                    continue;
                } else if at < slot_at {
                    holding.get(&colours[at - colour_at].0)
                } else {
                    reaching.get(&slots[at - slot_at].0)
                };
                reached.extend(dependents.into_iter().flatten());
            }
        }
        first_hour = false;
        for at in changed.drain(..) {
            marked[at] = false;
        }
        let built = values[0].filter(|&built| built >= 0).map(|built| built as u8);
        for index in reached {
            let link = values[tag_at + index].map(|value| ((value >> 32) as u32, value as u32));
            if link != links[index] {
                if let Some((id, _)) = links[index]
                    && let Some(holders) = holding.get_mut(&id)
                {
                    holders.remove(&index);
                }
                if let Some((id, _)) = link {
                    holding.entry(id).or_default().insert(index);
                }
                links[index] = link;
            }
            // The slots its name reaches now, overlord by overlord, each once.
            let mut chain = Vec::new();
            let mut next_slot = link.map(|(_, slot)| slot);
            while let Some(slot) = next_slot.filter(|slot| !chain.contains(slot)) {
                chain.push(slot);
                next_slot = overlord(&values, slot);
            }
            if chain != chains[index] {
                for slot in &chains[index] {
                    if let Some(named) = reaching.get_mut(slot) {
                        named.remove(&index);
                    }
                }
                for &slot in &chain {
                    reaching.entry(slot).or_default().insert(index);
                }
                chains[index] = chain;
            }
            let fact = match link {
                None => [-1, -1, reasons.number("not a country at this hour")],
                Some((id, slot)) => {
                    let mut inputs = StyleInputs {
                        reserved_built: built,
                        ..StyleInputs::default()
                    };
                    inputs.tags.insert(tags[index].0.clone(), (id, slot));
                    if let Some(value) = colour_of.get(&id).and_then(|&at| values[at]) {
                        inputs
                            .colors
                            .insert(id, [(value >> 16) as u8, (value >> 8) as u8, value as u8]);
                    }
                    if localisation.is_some() {
                        for &slot in &chains[index] {
                            if let Some(operands) = operands_at(&values, slot)? {
                                inputs.operands.insert(slot, operands);
                            }
                        }
                    }
                    let styles = HourStyles::build(inputs, localisation);
                    match styles.fact(tags[index].0) {
                        Ok((style, problem)) => [
                            style.name.as_deref().map_or(-1, |name| names.number(name)),
                            style.color.map_or(-1, |[red, green, blue]| {
                                (i32::from(red) << 16) | (i32::from(green) << 8) | i32::from(blue)
                            }),
                            problem.map_or(-1, |problem| reasons.number(problem)),
                        ],
                        Err(reason) => [-1, -1, reasons.number(&reason)],
                    }
                }
            };
            if shown[index] != Some(fact) {
                shown[index] = Some(fact);
                countries[index].push([hour, fact[0], fact[1], fact[2]]);
            }
        }
    }
    reply.names = names.texts;
    reply.reasons = reasons.texts;
    reply.countries = tags
        .into_iter()
        .zip(countries)
        .filter(|(_, runs)| !runs.is_empty())
        .map(|((tag, _), runs)| CountryFactRuns { tag: tag.clone(), runs })
        .collect();
    Ok(Some(reply))
}

/// The naming operands at `hour` of each tag `inputs` holds, then of each overlord its name
/// reaches.
fn operands(
    published: &Published,
    reader: &mut series::Reader<'_>,
    hour: i32,
    inputs: &mut StyleInputs,
) -> io::Result<()> {
    let mut pending: Vec<u32> = inputs.tags.values().map(|&(_, slot)| slot).collect();
    while !pending.is_empty() {
        pending.sort_unstable();
        pending.dedup();
        pending.retain(|slot| !inputs.operands.contains_key(slot));
        let slots: Vec<(u32, u64)> = pending
            .iter()
            .filter_map(|slot| published.slots.get(slot).map(|&key| (*slot, key)))
            .collect();
        let keyed: Vec<u64> = slots.iter().map(|&(_, key)| key).collect();
        let mut next = Vec::new();
        for (&(slot, _), value) in slots.iter().zip(reader.values_at(&keyed, hour)?) {
            let Some(value) = value else { continue };
            let operands = usize::try_from(value)
                .ok()
                .and_then(|at| published.operands.get(at))
                .ok_or_else(|| invalid("Prepared country naming operands are not catalogued"))?
                .as_ref()
                .map(Parts::operands)
                .map_err(String::clone);
            if let Ok(Some(overlord)) = operands.as_ref().map(|operands| operands.overlord) {
                next.push(overlord);
            }
            inputs.operands.insert(slot, operands);
        }
        pending = next;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An hour's values come in an order its content alone decides, so preparing one recording
    /// twice writes the same keys and series.
    #[test]
    fn an_hours_values_come_in_one_order() {
        use replay_data::rows::{CountryAppearanceRow, TableRows};
        let mut frame = crate::test_fixtures::scale_frame(1, 10, 64);
        *frame.tables[TableId::CountryAppearance.index()] =
            TableView::Complete(TableRows::CountryAppearance(
                (0..64)
                    .map(|index| CountryAppearanceRow {
                        country: 100_000 + index,
                        red: (index as f32 / 64.0).to_bits(),
                        ..Default::default()
                    })
                    .collect(),
            ));
        *frame.tables[TableId::ReservedCountryNaming.index()] =
            TableView::Complete(TableRows::ReservedCountryNaming(Vec::new()));
        let observed = || {
            let mut values = Values::default();
            assert!(
                StylesObserver::default()
                    .observe(FormatVersion::CURRENT, &frame, &mut values)
                    .unwrap()
            );
            values
                .iter()
                .map(|(name, value)| (name.to_owned(), value))
                .collect::<Vec<_>>()
        };
        let first = observed();
        assert!(first.len() > 128, "{}", first.len());
        assert_eq!(first, observed());
    }
}
