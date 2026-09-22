//! Country flags from the recorded content stack, never launcher state or a tag catalog.

use super::{LoadContext, Product, validate_tag};
use crate::{Source, content, invalid};
use installed_content::{ResolveOptions, content::ResolvedContent};
use replay::{
    container::{Replay, SeekResult},
    frame::{DecodedFrame, TableView},
    rows::{TableSchema, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub struct CountryFlags;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum CountryFlagSize {
    #[default]
    Full,
    Small,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CountryFlagParams {
    pub branch_id: String,
    pub hour: i32,
    pub tags: Vec<String>,
    #[serde(default)]
    pub size: CountryFlagSize,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CountryFlagImage {
    pub tag: String,
    pub asset: String,
    pub width: u32,
    pub height: u32,
    /// RGBA bytes, at most 128 pixels on the longer side.
    pub pixels: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CountryFlagReply {
    pub branch_id: String,
    pub hour: i32,
    pub size: CountryFlagSize,
    pub flags: Vec<CountryFlagImage>,
    pub warnings: Vec<String>,
}

impl Product for CountryFlags {
    const KEY: &'static str = "country-flags";
    type Params = CountryFlagParams;
    type Reply = CountryFlagReply;

    fn types() -> Vec<String> {
        vec![
            CountryFlagSize::decl(),
            CountryFlagImage::decl(),
            CountryFlagParams::decl(),
            CountryFlagReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.tags.is_empty() || params.tags.len() > 32 {
            return Err("Invalid country flag selection");
        }
        for tag in &params.tags {
            validate_tag(tag)?;
        }
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid country flag branch");
        }
        Ok(())
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let content = context.content()?;
        let state = source.state::<Option<FlagContext>>();
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        if !state
            .as_ref()
            .is_some_and(|cached| cached.matches(&content.options, params))
        {
            *state = Some(FlagContext::load(
                source.replay(),
                source.content(&content.options)?,
                content.options.clone(),
                content.allow_unverified,
                params,
                cancelled,
            )?);
        }
        state
            .as_ref()
            .expect("flag context loaded")
            .reply(params, cancelled)
    }
}

/// The rows a table holds this hour; an unavailable table holds none.
fn rows<T: TableSchema>(frame: &DecodedFrame) -> io::Result<&[T::Row]> {
    match &frame.tables[T::ID.index()] {
        TableView::Complete(rows) | TableView::Truncated { rows, .. } => {
            rows.rows::<T>().ok_or_else(|| {
                invalid(format!(
                    "hour {} {:?} contains another table's rows",
                    frame.hour,
                    T::ID
                ))
            })
        }
        TableView::Unavailable { .. } => Ok(&[]),
    }
}

pub struct FlagChoice {
    pub raw: u32,
    pub tag: String,
    pub candidates: Vec<String>,
}

/// Flag asset candidates per country at one hour, shared with the combat UI.
pub fn choices(frame: &DecodedFrame) -> io::Result<Vec<FlagChoice>> {
    let links: BTreeMap<_, _> = rows::<tables::CountryLinks>(frame)?
        .iter()
        .map(|r| (r.id, r.canonical_slot))
        .collect();
    let countries: BTreeMap<_, _> = rows::<tables::Countries>(frame)?
        .iter()
        .map(|r| {
            (
                r.canonical_slot,
                String::from_utf8_lossy(&r.tag).into_owned(),
            )
        })
        .collect();
    let parties: BTreeMap<_, _> = rows::<tables::PoliticalParties>(frame)?
        .iter()
        .map(|r| {
            (
                (r.country, r.id),
                String::from_utf8_lossy(&r.token).into_owned(),
            )
        })
        .collect();
    let politics: BTreeMap<_, _> = rows::<tables::Politics>(frame)?
        .iter()
        .filter_map(|r| Some((r.country, parties.get(&(r.country, r.ruling_party?))?)))
        .collect();
    let appearances: BTreeMap<_, _> = rows::<tables::CountryAppearance>(frame)?
        .iter()
        .filter(|r| !r.cosmetic_tag.is_empty())
        .map(|r| {
            (
                r.country,
                String::from_utf8_lossy(&r.cosmetic_tag).into_owned(),
            )
        })
        .collect();
    Ok(links
        .into_iter()
        .filter_map(|(raw, slot)| {
            let tag = countries.get(&slot)?;
            let candidates = flag_candidates(
                tag,
                appearances.get(&raw).map(String::as_str),
                politics.get(&raw).map(|party| party.as_str()),
            );
            Some(FlagChoice {
                raw,
                tag: tag.clone(),
                candidates,
            })
        })
        .collect())
}

fn flag_candidates(tag: &str, cosmetic: Option<&str>, ideology: Option<&str>) -> Vec<String> {
    let base = cosmetic.filter(|value| !value.is_empty()).unwrap_or(tag);
    let mut candidates = Vec::new();
    if let Some(ideology) = ideology {
        candidates.push(format!("{base}_{ideology}"));
    }
    candidates.push(base.to_owned());
    if base != tag {
        candidates.push(tag.to_owned());
    }
    candidates
}

pub struct FlagContext {
    options: ResolveOptions,
    content: Arc<ResolvedContent>,
    choices: Vec<FlagChoice>,
    branch_id: String,
    hour: i32,
    warnings: Vec<String>,
}

impl FlagContext {
    fn matches(&self, options: &ResolveOptions, request: &CountryFlagParams) -> bool {
        self.options.game_dir == options.game_dir
            && self.options.user_dir == options.user_dir
            && self.options.cache_dir == options.cache_dir
            && self.branch_id == request.branch_id
            && self.hour == request.hour
    }

    fn load(
        replay: Arc<Replay>,
        content: Arc<ResolvedContent>,
        options: ResolveOptions,
        allow_unverified: bool,
        request: &CountryFlagParams,
        cancelled: &AtomicBool,
    ) -> io::Result<Self> {
        let mut warnings = vec!["Flag artwork is resolved from the recording's game content; visual assets are outside the content hash coverage.".into()];
        content::check_replay(&replay, allow_unverified, &mut warnings)?;
        let branch = request
            .branch_id
            .parse::<u64>()
            .map_err(|_| invalid("Invalid flag branch"))?;
        if cancelled.load(Ordering::Acquire) {
            return Err(invalid("Flag request cancelled"));
        }
        let frame = match replay.seek(branch, request.hour)? {
            SeekResult::Frame(frame) => frame,
            SeekResult::Gap { reason, .. } => {
                return Err(invalid(format!(
                    "Flag identities unavailable at this hour: {reason}"
                )));
            }
        };
        let choices = choices(&frame)?;
        Ok(Self {
            options,
            content,
            choices,
            branch_id: request.branch_id.clone(),
            hour: request.hour,
            warnings,
        })
    }

    fn reply(
        &self,
        request: &CountryFlagParams,
        cancelled: &AtomicBool,
    ) -> io::Result<CountryFlagReply> {
        let mut flags = Vec::new();
        let mut warnings = self.warnings.clone();
        let directory = match request.size {
            CountryFlagSize::Full => "gfx/flags",
            CountryFlagSize::Small => "gfx/flags/small",
        };
        for tag in &request.tags {
            if cancelled.load(Ordering::Acquire) {
                return Err(invalid("Flag request cancelled"));
            }
            let Some(choice) = self.choices.iter().find(|choice| &choice.tag == tag) else {
                warnings.push(format!("{tag}: country identity unavailable at this hour"));
                continue;
            };
            let mut found = None;
            for key in &choice.candidates {
                if key.is_empty()
                    || key.len() > 256
                    || !key
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                {
                    return Err(invalid("Invalid recorded flag asset name"));
                }
                let asset = format!("{directory}/{key}.tga");
                let bytes = match self.content.layers.read_uncovered(&asset) {
                    Ok(bytes) => bytes,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(error),
                };
                // Bound allocations before the shared TGA decoder reads authored extents.
                if bytes.len() < 18 || bytes.len() > 8 * 1024 * 1024 {
                    return Err(invalid("Flag image exceeds its input budget"));
                }
                let width = u16::from_le_bytes([bytes[12], bytes[13]]);
                let height = u16::from_le_bytes([bytes[14], bytes[15]]);
                if width == 0 || height == 0 || width > 2048 || height > 2048 {
                    return Err(invalid("Flag image dimensions exceed their budget"));
                }
                let image = installed_content::image::tga::decode(&bytes)?;
                let scale = (image.width.max(image.height) as f64 / 128.0).max(1.0);
                let width = (image.width as f64 / scale).round().max(1.0) as u32;
                let height = (image.height as f64 / scale).round().max(1.0) as u32;
                let mut pixels = Vec::with_capacity((width * height * 4) as usize);
                for y in 0..height {
                    for x in 0..width {
                        let at = ((y * image.height / height * image.width
                            + x * image.width / width)
                            * 4) as usize;
                        pixels.extend_from_slice(&image.pixels[at..at + 4]);
                    }
                }
                found = Some(CountryFlagImage {
                    tag: tag.clone(),
                    asset,
                    width,
                    height,
                    pixels,
                });
                break;
            }
            if let Some(flag) = found {
                flags.push(flag);
            } else {
                warnings.push(format!(
                    "{tag}: flag artwork unavailable in the recorded content"
                ));
            }
        }
        Ok(CountryFlagReply {
            branch_id: request.branch_id.clone(),
            hour: request.hour,
            size: request.size,
            flags,
            warnings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mod_cosmetic_and_ideology_keys_precede_the_recorded_base_tag() {
        assert_eq!(
            flag_candidates("Z01", Some("MOD_REPUBLIC"), Some("custom_ideology")),
            ["MOD_REPUBLIC_custom_ideology", "MOD_REPUBLIC", "Z01"]
        );
        assert_eq!(
            flag_candidates("Z01", None, Some("custom_ideology")),
            ["Z01_custom_ideology", "Z01"]
        );
        assert_eq!(flag_candidates("Z01", Some(""), None), ["Z01"]);
    }

    #[test]
    fn flag_params_default_to_full_and_reject_paths_and_unbounded_batches() {
        let mut value = serde_json::json!({ "branchId": "1", "hour": 0, "tags": ["GER"] });
        let params: CountryFlagParams = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(params.size, CountryFlagSize::Full);
        assert!(CountryFlags::validate(&params).is_ok());
        value["size"] = serde_json::json!("small");
        let params: CountryFlagParams = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(params.size, CountryFlagSize::Small);
        value["size"] = serde_json::json!("../other");
        assert!(serde_json::from_value::<CountryFlagParams>(value).is_err());
        let mut params = CountryFlagParams {
            branch_id: "1".into(),
            hour: 0,
            tags: vec!["Z01".into(), "MOD_TAG".into()],
            size: CountryFlagSize::Full,
        };
        assert!(CountryFlags::validate(&params).is_ok());
        params.tags = vec!["../secret".into()];
        assert!(CountryFlags::validate(&params).is_err());
        params.tags = vec!["Z01".into(); 33];
        assert!(CountryFlags::validate(&params).is_err());
    }
}
