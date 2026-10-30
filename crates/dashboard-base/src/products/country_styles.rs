//! Each country's name and map colour as the game showed them at one hour, for
//! chart series, legends and labels.
//!
//! Names are the engine-built names (`CountryNames`: cosmetic, ideology and
//! subject variants) in the user's game language; colours are the recorded
//! stored map colour. Neither comes from a tag list, so mods and dynamic
//! countries work, and a missing value stays missing rather than guessed.

use super::{ContentOptions, LoadContext, Product, recorded_rows, validate_tag};
use crate::{Source, catalogue::plain_text, content, invalid};
use installed_content_country_names::country_names::{CountryNames, Operands};
use installed_content_localisation::localisation::Localisation;
use replay::container::SeekResult;
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub struct CountryStyles;

/// The chart's plain country name, shared with the offline content check.
pub fn country_name(names: &CountryNames<'_>, slot: u32) -> Result<String, String> {
    names
        .name(slot)
        .and_then(|name| plain_text(&name).map_err(|reason| format!("its name holds {reason}")))
}

/// Most countries one request may name.
const MAX_TAGS: usize = 64;
/// Hours kept per source, so a chart's legend and its labels share one decode.
const RECENT_HOURS: usize = 4;
/// The tables an hour's styles are read from; the seek adds the campaign.
pub(crate) const STYLE_TABLES: &[TableId] = &installed_content_country_names::country_names::TABLES;

/// One hour's frame for its styles, or the gap that hour is. Every style product
/// reads hours this way, so they agree on which hours can be read.
pub(crate) fn seek_styles(
    source: &Source,
    branch: u64,
    hour: i32,
    cancelled: &AtomicBool,
) -> io::Result<SeekResult> {
    crate::prepared::query::frames::seek(source, branch, hour, STYLE_TABLES, cancelled)
}

/// Each hour's style inputs as preparation keeps them, which the dashboard installs once
/// ([`install_style_runs`]): a request whose hours they all hold answers without a seek.
pub trait StyleRuns: Sync {
    /// Visits the inputs of `tags` at each of the ascending, distinct `hours` on `branch` in
    /// turn, with their naming operands when `names` are read: false, visiting none, unless
    /// preparation holds every one of those hours.
    #[allow(clippy::too_many_arguments)]
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
    ) -> io::Result<bool>;
}

static RUNS: OnceLock<&'static dyn StyleRuns> = OnceLock::new();

/// Installs preparation's style inputs; later installs keep the first.
pub fn install_style_runs(runs: &'static dyn StyleRuns) {
    let _ = RUNS.set(runs);
}

/// [`StyleRuns::inputs`] of the installed runs; false without them.
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepared_inputs(
    source: &Source,
    content: Option<&ContentOptions>,
    branch: u64,
    hours: &[i32],
    tags: &[String],
    names: bool,
    cancelled: &AtomicBool,
    visit: &mut dyn FnMut(i32, StyleInputs) -> io::Result<()>,
) -> io::Result<bool> {
    match RUNS.get() {
        Some(runs) => runs.inputs(
            source, content, branch, hours, tags, names, cancelled, visit,
        ),
        None => Ok(false),
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CountryStyleParams {
    pub branch_id: String,
    pub hour: i32,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct CountryStyle {
    pub tag: String,
    /// The name the game showed; null when it could not be resolved.
    pub name: Option<String>,
    /// The recorded map colour as sRGB bytes; null when not recorded this hour.
    pub color: Option<[u8; 3]>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CountryStyleReply {
    pub branch_id: String,
    pub hour: i32,
    pub styles: Vec<CountryStyle>,
    pub warnings: Vec<String>,
}

/// The product naming every country's styles over every hour preparation holds.
pub const FACTS_PRODUCT: &str = "country-facts";

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CountryFactsParams {
    pub branch_id: String,
}

/// Every country's style ([`CountryStyle`]) at every hour preparation read, as runs, so an
/// hour is looked up rather than asked for. An hour no `known` span holds is `country-styles`'.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct CountryFactsReply {
    pub branch_id: String,
    /// The hours the runs answer, as `[first, last]` spans in order.
    pub known: Vec<[i32; 2]>,
    /// Names, each once; a run holds its index.
    pub names: Vec<String>,
    /// Why part of a style is missing, each once; a run holds its index.
    pub reasons: Vec<String>,
    pub countries: Vec<CountryFactRuns>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct CountryFactRuns {
    pub tag: String,
    /// `[hour, name, colour, reason]` from `hour` until the next run: the name's index in
    /// `names`, the colour as `0xRRGGBB` and the reason's index in `reasons`, each -1 when
    /// there is none. A tag that is not a country has a run with no name or colour and its reason.
    pub runs: Vec<[i32; 4]>,
}

impl HourStyles {
    /// `tag`'s style this hour and why part of it is missing, or, when it is not a country,
    /// why not.
    pub fn fact(&self, tag: &str) -> Result<(&CountryStyle, Option<&str>), String> {
        self.style(tag).ok_or_else(|| self.missing_reason(tag))
    }
}

impl Product for CountryStyles {
    const KEY: &'static str = "country-styles";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = CountryStyleParams;
    type Reply = CountryStyleReply;

    fn types() -> Vec<String> {
        vec![
            CountryStyleParams::decl(),
            CountryStyle::decl(),
            CountryStyleReply::decl(),
            CountryFactsParams::decl(),
            CountryFactRuns::decl(),
            CountryFactsReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.tags.is_empty() || params.tags.len() > MAX_TAGS {
            return Err("Invalid country style selection");
        }
        for tag in &params.tags {
            validate_tag(tag)?;
        }
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid country style branch");
        }
        Ok(())
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let localisation = content::name_localisation(source, context.content.as_ref());
        let key = HourKey {
            branch_id: params.branch_id.clone(),
            hour: params.hour,
            localisation: localisation.as_ref().ok().cloned(),
        };
        let shared = source.state::<StyleState>();
        let cached = {
            let mut state = shared.lock().unwrap_or_else(|error| error.into_inner());
            state
                .recent
                .iter()
                .position(|(cached, _)| cached.matches(&key))
                .map(|index| {
                    let entry = state.recent.remove(index);
                    state.recent.insert(0, entry);
                    Arc::clone(&state.recent[0].1)
                })
        };
        // The lock is not held while reading the recording, so other hours wait for nothing.
        let hour = match cached {
            Some(hour) => hour,
            None => {
                if cancelled.load(Ordering::Acquire) {
                    return Err(invalid("Country style request cancelled"));
                }
                let branch = params
                    .branch_id
                    .parse::<u64>()
                    .map_err(|_| invalid("Invalid country style branch"))?;
                // Preparation's runs give the requested tags' share of the hour, kept for no
                // other request.
                let mut prepared = None;
                prepared_inputs(
                    source,
                    context.content.as_ref(),
                    branch,
                    &[params.hour],
                    &params.tags,
                    key.localisation.is_some(),
                    cancelled,
                    &mut |_, inputs| {
                        prepared = Some(inputs);
                        Ok(())
                    },
                )?;
                if let Some(inputs) = prepared {
                    Arc::new(HourStyles::build(inputs, key.localisation.as_deref()))
                } else {
                    let frame = match seek_styles(source, branch, params.hour, cancelled)? {
                        SeekResult::Frame(frame) => frame,
                        SeekResult::Gap { reason, .. } => {
                            return Err(invalid(format!(
                                "Country styles unavailable at this hour: {reason}"
                            )));
                        }
                    };
                    let hour = Arc::new(HourStyles::read(&frame, key.localisation.as_deref())?);
                    let mut state = shared.lock().unwrap_or_else(|error| error.into_inner());
                    state.recent.insert(0, (key, Arc::clone(&hour)));
                    state.recent.truncate(RECENT_HOURS);
                    hour
                }
            }
        };
        let mut warnings = Vec::new();
        match (&localisation, context.content.as_ref()) {
            (Err(reason), _) => warnings.push(reason.clone()),
            // Names read from unverified content keep its standing warning.
            (Ok(_), Some(content)) => {
                content::check_replay(&source.replay(), content.allow_unverified, &mut warnings)?
            }
            (Ok(_), None) => {}
        }
        let styles = params
            .tags
            .iter()
            .map(|tag| match hour.by_tag.get(tag) {
                Some((style, problem)) => {
                    if let Some(problem) = problem {
                        warnings.push(format!("{tag}: {problem}"));
                    }
                    style.clone()
                }
                None => {
                    warnings.push(format!("{tag}: {}", hour.missing_reason(tag)));
                    CountryStyle {
                        tag: tag.clone(),
                        name: None,
                        color: None,
                    }
                }
            })
            .collect();
        Ok(CountryStyleReply {
            branch_id: params.branch_id.clone(),
            hour: params.hour,
            styles,
            warnings,
        })
    }
}

/// One hour, named with one localisation. The localisation is compared by
/// identity: a new language or a settings file that became readable loads a new
/// one, so the hour is named again. Holding it keeps its address from being reused.
struct HourKey {
    branch_id: String,
    hour: i32,
    localisation: Option<Arc<Localisation>>,
}

impl HourKey {
    fn matches(&self, other: &HourKey) -> bool {
        self.branch_id == other.branch_id
            && self.hour == other.hour
            && match (&self.localisation, &other.localisation) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            }
    }
}

#[derive(Default)]
struct StyleState {
    /// Most recently used first.
    recent: Vec<(HourKey, Arc<HourStyles>)>,
}

/// What one hour's styles are built from ([`HourStyles::build`]): as a frame records them
/// ([`Self::read`]), or as preparation's runs give them back for the tags a request names.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StyleInputs {
    /// Each tag's first country row: its raw link id and canonical slot. Ordered, as preparation
    /// keeps them in the order they are visited and its output is a function of the recording.
    pub tags: BTreeMap<String, (u32, u32)>,
    /// Each raw link id's recorded map colour, ordered as `tags` are.
    pub colors: BTreeMap<u32, [u8; 3]>,
    /// Why the appearance table is incomplete this hour, if it is.
    pub appearance: Option<String>,
    /// Why the country table is incomplete this hour, if it is.
    pub incomplete: Option<String>,
    /// The naming operands ([`CountryNames::recorded_operands`]): every country's, or at least
    /// those of each tag's slot and of every overlord its name reaches. None are read without
    /// names.
    pub operands: BTreeMap<u32, Result<Operands, String>>,
    pub reserved_built: Option<u8>,
}

impl StyleInputs {
    /// The inputs `frame` records, with their naming operands when `names` are read.
    pub fn read(frame: &DecodedFrame, names: bool) -> io::Result<Self> {
        let colors = recorded_rows::<tables::CountryAppearance>(frame)?
            .iter()
            .map(|row| (row.country, row.map_color()))
            .collect();
        let appearance = frame
            .complete::<tables::CountryAppearance>()
            .err()
            .map(|error| error.to_string());
        // Only a complete country table can say a tag is not a country this hour.
        let incomplete = frame
            .complete::<tables::Countries>()
            .err()
            .map(|error| error.to_string());
        let mut tags = BTreeMap::new();
        // Countries are keyed by their raw link id, which appearance rows share.
        for country in recorded_rows::<tables::Countries>(frame)? {
            tags.entry(String::from_utf8_lossy(&country.tag).into_owned())
                .or_insert((country.id, country.canonical_slot));
        }
        let (operands, reserved_built) = if names {
            CountryNames::recorded_operands(frame)
        } else {
            (BTreeMap::new(), None)
        };
        Ok(Self {
            tags,
            colors,
            appearance,
            incomplete,
            operands,
            reserved_built,
        })
    }
}

/// Every country's style at one hour, with why a part of it is missing.
pub struct HourStyles {
    by_tag: HashMap<String, (CountryStyle, Option<String>)>,
    /// Why the country table was not complete this hour, if it was not.
    incomplete: Option<String>,
}

impl HourStyles {
    /// Reads names and colours from one frame. Without localisation every name
    /// is null and the reply says why.
    pub fn read(frame: &DecodedFrame, localisation: Option<&Localisation>) -> io::Result<Self> {
        Ok(Self::build(
            StyleInputs::read(frame, localisation.is_some())?,
            localisation,
        ))
    }

    /// The styles of the tags `inputs` hold, named with `localisation`.
    pub fn build(inputs: StyleInputs, localisation: Option<&Localisation>) -> Self {
        let StyleInputs {
            tags,
            colors,
            appearance,
            incomplete,
            operands,
            reserved_built,
        } = inputs;
        let names = localisation
            .map(|localisation| CountryNames::recorded(localisation, operands, reserved_built));
        // A colour the appearance table may have omitted says so.
        let no_colour = match appearance {
            None => "no recorded map colour".to_owned(),
            Some(error) => format!(
                "no recorded map colour (the appearance table is incomplete at this hour: {error})"
            ),
        };
        let by_tag = tags
            .into_iter()
            .map(|(tag, (id, slot))| {
                let (name, name_problem) = match &names {
                    None => (None, None),
                    // Names are plain text for charts: the game's colour codes are removed.
                    Some(names) => match country_name(names, slot) {
                        Ok(name) => (Some(name), None),
                        Err(reason) => (None, Some(reason)),
                    },
                };
                let color = colors.get(&id).copied();
                let problem = match (name_problem, color) {
                    (Some(reason), None) => Some(format!("{reason}; {no_colour}")),
                    (Some(reason), Some(_)) => Some(reason),
                    (None, None) => Some(no_colour.clone()),
                    (None, Some(_)) => None,
                };
                (tag.clone(), (CountryStyle { tag, name, color }, problem))
            })
            .collect();
        Self { by_tag, incomplete }
    }

    /// A tag's style this hour and why part of it is missing, if it is a country then.
    pub fn style(&self, tag: &str) -> Option<(&CountryStyle, Option<&str>)> {
        self.by_tag
            .get(tag)
            .map(|(style, problem)| (style, problem.as_deref()))
    }

    /// Why a tag has no style this hour.
    pub fn missing_reason(&self, _tag: &str) -> String {
        match &self.incomplete {
            Some(reason) => format!("the country table is incomplete at this hour ({reason})"),
            None => "not a country at this hour".to_owned(),
        }
    }

    pub fn get(&self, tag: &str) -> Option<&CountryStyle> {
        self.by_tag.get(tag).map(|(style, _)| style)
    }

    /// A tag's recorded map colour, or why it has none, as this hour's warnings say.
    pub fn colour(&self, tag: &str) -> Result<[u8; 3], String> {
        match self.by_tag.get(tag) {
            Some((style, problem)) => style
                .color
                .ok_or_else(|| problem.clone().unwrap_or_default()),
            None => Err(self.missing_reason(tag)),
        }
    }
}

#[cfg(test)]
mod tests;
