//! Each country's name and map colour as the game showed them at one hour, for
//! chart series, legends and labels.
//!
//! Names are the engine-built names (`CountryNames`: cosmetic, ideology and
//! subject variants) in the user's game language; colours are the recorded
//! stored map colour. Neither comes from a tag list, so mods and dynamic
//! countries work, and a missing value stays missing rather than guessed.

use super::{LoadContext, Product, recorded_rows, validate_tag};
use crate::{Source, catalogue::plain_text, content, invalid};
use installed_content_country_names::country_names::CountryNames;
use installed_content_localisation::localisation::Localisation;
use replay::container::SeekResult;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io,
    sync::{
        Arc,
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
                let tables = Some(&installed_content_country_names::country_names::TABLES[..]);
                let frame = match source.replay().seek_selected(branch, params.hour, tables)? {
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
        let names = localisation.map(|localisation| CountryNames::read(frame, localisation));
        let colors: HashMap<u32, [u8; 3]> = recorded_rows::<tables::CountryAppearance>(frame)?
            .iter()
            .map(|row| (row.country, row.map_color()))
            .collect();
        let mut by_tag = HashMap::new();
        // Only a complete country table can say a tag is not a country this hour.
        let incomplete = frame
            .complete::<tables::Countries>()
            .err()
            .map(|error| error.to_string());
        // Countries are keyed by their raw link id, which appearance rows share.
        for country in recorded_rows::<tables::Countries>(frame)? {
            let tag = String::from_utf8_lossy(&country.tag).into_owned();
            let (name, name_problem) = match &names {
                None => (None, None),
                // Names are plain text for charts: the game's colour codes are removed.
                Some(names) => match country_name(names, country.canonical_slot) {
                    Ok(name) => (Some(name), None),
                    Err(reason) => (None, Some(reason)),
                },
            };
            let color = colors.get(&country.id).copied();
            let problem = match (name_problem, color) {
                (Some(reason), None) => Some(format!("{reason}; no recorded map colour")),
                (Some(reason), Some(_)) => Some(reason),
                (None, None) => Some("no recorded map colour".to_owned()),
                (None, Some(_)) => None,
            };
            by_tag
                .entry(tag.clone())
                .or_insert((CountryStyle { tag, name, color }, problem));
        }
        Ok(Self { by_tag, incomplete })
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
}

#[cfg(test)]
mod tests;
