//! Country styles prepared as a segment family on the walk's world ([`facts`]): each tag's
//! holder, map colour, naming operands and flag choice kept as runs on the family's series
//! publication ([`emit`]). `country-facts` reads every tag's runs whole ([`country_facts`]),
//! and `country-styles` and `country-colours` the hours they ask ([`PreparedStyles`]); names
//! are built at the reply, in the reader's language. Neither reads the recording.

use crate::{
    content, invalid, metadata::check_cancel, prepared::families,
    products::country_flags::FlagFolders,
};
use dashboard_base::{
    Source,
    prepared::engine,
    products::{
        ContentOptions,
        country_styles::{
            CountryFactRuns, CountryFactsReply, CountryFlagChoice, HourStyles, StyleInputs,
            StyleRuns,
        },
    },
};
use dashboard_series::{PageRead, Run, Snapshot, family::Directory};
use emit::{CHOICES, COLOUR, FAMILY, FLAG, HOLDER, KNOWN, NAMING, NAMINGS};
use installed_content_localisation::localisation::Localisation;
use std::{
    collections::HashMap,
    io,
    sync::{Arc, atomic::AtomicBool},
};

pub mod emit;
pub mod facts;
#[cfg(test)]
mod tests;

/// The family's name: its publication's and coverage's, which gates every style product.
pub const NAME: &str = "country-styles";

/// What country styles are prepared under: the recorded mod stack's flag folders, or why flags
/// cannot be chosen there.
#[derive(Clone)]
pub struct Configured {
    pub(crate) flags: Result<Arc<FlagFolders>, String>,
}

impl Configured {
    /// Country styles of `source` under `options`; a refused content is named, never an error.
    pub fn read(source: &Source, options: Option<&ContentOptions>) -> io::Result<Self> {
        let flags = match options
            .ok_or_else(|| invalid("Game content is not configured"))
            .and_then(|options| {
                content::check_replay(
                    source.replay_ref(),
                    options.allow_unverified,
                    &mut Vec::new(),
                )?;
                source.content(&options.options)
            })
            .and_then(|content| FlagFolders::read(&content))
        {
            Ok(folders) => Ok(Arc::new(folders)),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
            Err(error) => Err(format!("flags cannot be chosen: {error}")),
        };
        Ok(Self { flags })
    }

    /// Country styles choosing flags in `flags`, or naming why they cannot.
    #[cfg(test)]
    pub(crate) fn with(flags: Result<FlagFolders, String>) -> Self {
        Self {
            flags: flags.map(Arc::new),
        }
    }

    /// One segment's projection, counting its full rebuilds into `rebuilds`.
    pub fn projector(&self, rebuilds: &Arc<facts::Rebuilds>) -> io::Result<facts::Projector> {
        facts::Projector::new(Arc::clone(rebuilds), self.flags.as_ref().ok().cloned())
    }

    /// The family's emit on a branch, from its first hour.
    pub fn emit(&self) -> emit::Emit {
        emit::Emit::new(self)
    }
}

/// The hours the publication knows within `first..=last`, as `[first, last]` spans in order.
fn known<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    [first, last]: [i32; 2],
) -> io::Result<Vec<[i32; 2]>> {
    let mut spans: Vec<[i32; 2]> = Vec::new();
    let Some(key) = directory.key(FAMILY, KNOWN, None) else {
        return Ok(spans);
    };
    for run in engine::runs(snapshot, key, first, last)? {
        match spans.last_mut() {
            Some(span) if span[1].checked_add(1) == Some(run.span.first) => span[1] = run.span.last,
            _ => spans.push([run.span.first, run.span.last]),
        }
    }
    Ok(spans)
}

/// Whether `spans` hold `hour`.
fn held(spans: &[[i32; 2]], hour: i32) -> bool {
    let at = spans.partition_point(|span| span[1] < hour);
    spans.get(at).is_some_and(|span| span[0] <= hour)
}

/// Each of `actor`'s runs over `first..=last`, by measure ([`HOLDER`] to [`FLAG`]).
fn runs<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    actor: u64,
    measures: usize,
    [first, last]: [i32; 2],
) -> io::Result<Vec<Vec<Run>>> {
    (HOLDER..)
        .take(measures)
        .map(|measure| match directory.key(actor, measure, None) {
            Some(key) => engine::runs(snapshot, key, first, last),
            None => Ok(Vec::new()),
        })
        .collect()
}

/// Each measure's value at ascending hours, from its runs.
struct Sweep {
    runs: Vec<Vec<Run>>,
    next: Vec<usize>,
}

impl Sweep {
    fn new(runs: Vec<Vec<Run>>) -> Self {
        Self {
            next: vec![0; runs.len()],
            runs,
        }
    }

    /// Each measure's value at `hour`, no earlier than the hour asked before.
    fn at(&mut self, hour: i32) -> Vec<Option<i128>> {
        self.runs
            .iter()
            .zip(&mut self.next)
            .map(|(runs, next)| {
                while runs.get(*next).is_some_and(|run| run.span.last < hour) {
                    *next += 1;
                }
                runs.get(*next)
                    .filter(|run| run.span.first <= hour)
                    .map(|run| run.value)
            })
            .collect()
    }
}

/// The naming operands numbered `number`, decoded once per reply.
fn naming<'n>(
    namings: &'n mut HashMap<i128, Arc<facts::Naming>>,
    directory: &Directory,
    number: i128,
) -> io::Result<&'n Arc<facts::Naming>> {
    if !namings.contains_key(&number) {
        let cell = u64::try_from(number)
            .ok()
            .and_then(|number| directory.scalar(NAMINGS + number))
            .ok_or_else(|| invalid("Prepared country naming operands are not catalogued"))?;
        namings.insert(number, Arc::new(emit::naming_of(cell)?));
    }
    Ok(&namings[&number])
}

/// `inputs` holding `tag`'s holder, colour and naming operands from `values` (its runs' values
/// by measure at an hour), its operands only when names are read.
fn add_style(
    inputs: &mut StyleInputs,
    tag: &str,
    values: &[Option<i128>],
    naming: Option<&facts::Naming>,
) {
    let Some(holder) = values[HOLDER as usize] else {
        return;
    };
    let id = (holder >> 32) as u32;
    inputs.tags.insert(tag.to_owned(), (id, holder as u32));
    if let Some(value) = values[COLOUR as usize] {
        inputs
            .colors
            .insert(id, [(value >> 16) as u8, (value >> 8) as u8, value as u8]);
    }
    if let Some(naming) = naming {
        for (slot, operands) in &naming.chain {
            inputs.operands.insert(*slot, operands.clone());
        }
        if naming.built.is_some() {
            inputs.reserved_built = naming.built;
        }
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

/// Every tag's style and flag at every hour `branch`'s publication knows from `first` through
/// `last`, named with `localisation` ([`CountryFactsReply`]); None before the family publishes
/// the branch.
pub(crate) fn country_facts(
    source: &Source,
    content: Option<&ContentOptions>,
    branch: u64,
    hours: [i32; 2],
    localisation: Option<&Localisation>,
    cancelled: &AtomicBool,
) -> io::Result<Option<CountryFactsReply>> {
    let Some(opened) = families::open(source, content, NAME, emit::SCHEMA, branch)? else {
        return Ok(None);
    };
    opened
        .read(cancelled, |snapshot, directory| {
            facts_reply(snapshot, directory, branch, hours, localisation, cancelled)
        })
        .map(Some)
}

/// [`country_facts`] from the publication `snapshot` pins, its directory `directory`. A tag's
/// facts are given at each known span's first hour and at every known hour one of its values
/// changes; a change in a gap is given at the next span's first hour.
pub(crate) fn facts_reply<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    branch: u64,
    [first, last]: [i32; 2],
    localisation: Option<&Localisation>,
    cancelled: &AtomicBool,
) -> io::Result<CountryFactsReply> {
    let mut reply = CountryFactsReply {
        branch_id: branch.to_string(),
        ..CountryFactsReply::default()
    };
    if last < first {
        return Ok(reply);
    }
    reply.known = known(snapshot, directory, [first, last])?;
    let mut tags: Vec<(&str, u64)> = (0_u64..)
        .zip(&directory.actors)
        .filter_map(|(actor, named)| Some((std::str::from_utf8(&named.name).ok()?, actor)))
        .collect();
    tags.sort_unstable();
    let mut names = Dictionary::default();
    let mut reasons = Dictionary::default();
    let mut namings = HashMap::new();
    let mut flags: HashMap<i128, i32> = HashMap::new();
    // A tag's facts follow from its values alone, whichever tag holds them.
    let mut facts: HashMap<Vec<Option<i128>>, [i32; 4]> = HashMap::new();
    for (tag, actor) in tags {
        check_cancel(cancelled)?;
        let runs = runs(snapshot, directory, actor, 4, [first, last])?;
        let mut hours: Vec<i32> = reply.known.iter().map(|span| span[0]).collect();
        for run in runs.iter().flatten() {
            hours.push(run.span.first);
            hours.extend(run.span.last.checked_add(1));
        }
        hours.retain(|&hour| held(&reply.known, hour));
        hours.sort_unstable();
        hours.dedup();
        let mut sweep = Sweep::new(runs);
        let mut shown = None;
        let mut given = Vec::new();
        for hour in hours {
            let values = sweep.at(hour);
            let fact = match facts.get(&values).copied() {
                Some(fact) => fact,
                None => {
                    let fact = match values[HOLDER as usize] {
                        None => [-1, -1, reasons.number("not a country at this hour"), -1],
                        Some(_) => {
                            let naming = match (localisation, values[NAMING as usize]) {
                                (Some(_), Some(number)) => {
                                    Some(Arc::clone(naming(&mut namings, directory, number)?))
                                }
                                _ => None,
                            };
                            let mut inputs = StyleInputs::default();
                            add_style(&mut inputs, tag, &values, naming.as_deref());
                            let styles = HourStyles::build(inputs, localisation);
                            let [name, colour, reason] = match styles.fact(tag) {
                                Ok((style, problem)) => [
                                    style.name.as_deref().map_or(-1, |name| names.number(name)),
                                    style.color.map_or(-1, |[red, green, blue]| {
                                        (i32::from(red) << 16)
                                            | (i32::from(green) << 8)
                                            | i32::from(blue)
                                    }),
                                    problem.map_or(-1, |problem| reasons.number(problem)),
                                ],
                                Err(reason) => [-1, -1, reasons.number(&reason)],
                            };
                            let flag = match values[FLAG as usize] {
                                None => -1,
                                Some(number) => match flags.get(&number) {
                                    Some(&flag) => flag,
                                    None => {
                                        let choice = choice(directory, number)?;
                                        let flag = reply.flags.len() as i32;
                                        reply.flags.push(choice);
                                        flags.insert(number, flag);
                                        flag
                                    }
                                },
                            };
                            [name, colour, reason, flag]
                        }
                    };
                    facts.insert(values, fact);
                    fact
                }
            };
            if shown != Some(fact) {
                shown = Some(fact);
                given.push([hour, fact[0], fact[1], fact[2], fact[3]]);
            }
        }
        if !given.is_empty() {
            reply.countries.push(CountryFactRuns {
                tag: tag.to_owned(),
                runs: given,
            });
        }
    }
    reply.names = names.texts;
    reply.reasons = reasons.texts;
    Ok(reply)
}

/// The flag choice numbered `number`.
fn choice(directory: &Directory, number: i128) -> io::Result<CountryFlagChoice> {
    let cell = u64::try_from(number)
        .ok()
        .and_then(|number| directory.scalar(CHOICES + number))
        .ok_or_else(|| invalid("Prepared country flag choices are not catalogued"))?;
    emit::choice_of(cell)
}

/// Preparation's style inputs, which the style products read through [`StyleRuns`].
pub struct PreparedStyles;

pub static PREPARED_STYLES: PreparedStyles = PreparedStyles;

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
        if hours.is_empty() {
            return Ok(true);
        }
        let Some(opened) = families::open(source, content, NAME, emit::SCHEMA, branch)? else {
            return Ok(false);
        };
        opened.read(cancelled, |snapshot, directory| {
            styles_at(snapshot, directory, hours, tags, names, cancelled, visit)
        })
    }
}

/// [`StyleRuns::inputs`] from the publication `snapshot` pins: false, visiting none, unless it
/// knows every one of `hours`.
pub(crate) fn styles_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    hours: &[i32],
    tags: &[String],
    names: bool,
    cancelled: &AtomicBool,
    visit: &mut dyn FnMut(i32, StyleInputs) -> io::Result<()>,
) -> io::Result<bool> {
    let (Some(&first), Some(&last)) = (hours.first(), hours.last()) else {
        return Ok(true);
    };
    let spans = known(snapshot, directory, [first, last])?;
    if !hours.iter().all(|&hour| held(&spans, hour)) {
        return Ok(false);
    }
    let measures = if names { 3 } else { 2 };
    let mut sweeps = Vec::with_capacity(tags.len());
    for tag in tags {
        if let Some(actor) = directory.actor(tag.as_bytes()) {
            let runs = runs(snapshot, directory, actor, measures, [first, last])?;
            sweeps.push((tag.as_str(), Sweep::new(runs)));
        }
    }
    let mut namings = HashMap::new();
    for &hour in hours {
        check_cancel(cancelled)?;
        let mut inputs = StyleInputs::default();
        for (tag, sweep) in &mut sweeps {
            let values = sweep.at(hour);
            let naming = match values.get(NAMING as usize).copied().flatten() {
                Some(number) => Some(Arc::clone(naming(&mut namings, directory, number)?)),
                None => None,
            };
            add_style(&mut inputs, *tag, &values, naming.as_deref());
        }
        visit(hour, inputs)?;
    }
    Ok(true)
}
