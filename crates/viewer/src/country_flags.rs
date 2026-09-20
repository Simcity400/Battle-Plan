//! Country flags from the recorded content stack, never launcher state or a tag catalog.
use crate::{playback::Playback, table_access::rows};
use desktop_protocol::dashboard_data::{
    CountryFlagImage, CountryFlagReply, CountryFlagRequest, CountryFlagSize,
};
use installed_content::{ResolveOptions, content::ResolvedContent};
use replay::{frame::DecodedFrame, rows::tables};
use std::{
    collections::BTreeMap,
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub(crate) struct FlagChoice {
    pub raw: u32,
    pub tag: String,
    pub candidates: Vec<String>,
}

pub(crate) fn choices(frame: &DecodedFrame) -> io::Result<Vec<FlagChoice>> {
    let links: BTreeMap<_, _> = rows::<tables::CountryLinks>(frame)?
        .rows()
        .iter()
        .map(|r| (r.id, r.canonical_slot))
        .collect();
    let countries: BTreeMap<_, _> = rows::<tables::Countries>(frame)?
        .rows()
        .iter()
        .map(|r| {
            (
                r.canonical_slot,
                String::from_utf8_lossy(&r.tag).into_owned(),
            )
        })
        .collect();
    let parties: BTreeMap<_, _> = rows::<tables::PoliticalParties>(frame)?
        .rows()
        .iter()
        .map(|r| {
            (
                (r.country, r.id),
                String::from_utf8_lossy(&r.token).into_owned(),
            )
        })
        .collect();
    let politics: BTreeMap<_, _> = rows::<tables::Politics>(frame)?
        .rows()
        .iter()
        .filter_map(|r| Some((r.country, parties.get(&(r.country, r.ruling_party?))?)))
        .collect();
    let appearances: BTreeMap<_, _> = rows::<tables::CountryAppearance>(frame)?
        .rows()
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

pub(crate) struct FlagContext {
    options: ResolveOptions,
    content: Arc<ResolvedContent>,
    choices: Vec<FlagChoice>,
    branch_id: String,
    hour: i32,
    warnings: Vec<String>,
}

impl FlagContext {
    pub(crate) fn matches(&self, options: &ResolveOptions, request: &CountryFlagRequest) -> bool {
        self.options.game_dir == options.game_dir
            && self.options.user_dir == options.user_dir
            && self.options.cache_dir == options.cache_dir
            && self.branch_id == request.branch_id
            && self.hour == request.hour
    }

    pub(crate) fn load(
        replay: Arc<replay::container::Replay>,
        content: Arc<ResolvedContent>,
        options: ResolveOptions,
        allow_unverified: bool,
        request: &CountryFlagRequest,
        cancelled: &AtomicBool,
    ) -> io::Result<Self> {
        let mut warnings = vec!["Flag artwork is resolved from the recording's game content; visual assets are outside the content hash coverage.".into()];
        crate::content_verification::check_verification(
            "recording",
            &replay.identity.verification,
            allow_unverified,
            &mut warnings,
        )?;
        for session in &replay.sessions {
            crate::content_verification::check_verification(
                "session",
                &session.identity.verification,
                allow_unverified,
                &mut warnings,
            )?;
        }
        let branch = request
            .branch_id
            .parse::<u64>()
            .map_err(|_| crate::invalid("Invalid flag branch"))?;
        let mut playback = Playback::unloaded(Arc::clone(&replay))?;
        let (first, last) = playback
            .range(branch)
            .ok_or_else(|| crate::invalid("Flag branch unavailable"))?;
        if request.hour < first || request.hour > last {
            return Err(crate::invalid("Flag hour unavailable"));
        }
        if cancelled.load(Ordering::Acquire) {
            return Err(crate::invalid("Flag request cancelled"));
        }
        playback.seek(branch, request.hour)?;
        let frame = playback
            .frame()
            .ok_or_else(|| crate::invalid("Flag identities unavailable at this hour"))?;
        let choices = choices(frame)?;
        Ok(Self {
            options,
            content,
            choices,
            branch_id: request.branch_id.clone(),
            hour: request.hour,
            warnings,
        })
    }

    pub(crate) fn reply(
        &self,
        request: &CountryFlagRequest,
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
                return Err(crate::invalid("Flag request cancelled"));
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
                    return Err(crate::invalid("Invalid recorded flag asset name"));
                }
                let asset = format!("{directory}/{key}.tga");
                let bytes = match self.content.layers.read_uncovered(&asset) {
                    Ok(bytes) => bytes,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(error),
                };
                // Bound allocations before the shared TGA decoder reads authored extents.
                if bytes.len() < 18 || bytes.len() > 8 * 1024 * 1024 {
                    return Err(crate::invalid("Flag image exceeds its input budget"));
                }
                let width = u16::from_le_bytes([bytes[12], bytes[13]]);
                let height = u16::from_le_bytes([bytes[14], bytes[15]]);
                if width == 0 || height == 0 || width > 2048 || height > 2048 {
                    return Err(crate::invalid("Flag image dimensions exceed their budget"));
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
            source_id: request.source_id.clone(),
            revision: request.revision.clone(),
            branch_id: request.branch_id.clone(),
            hour: request.hour,
            flags,
            warnings,
            size: request.size,
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
}
