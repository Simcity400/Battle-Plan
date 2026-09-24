//! Country flags from the recorded content stack, never launcher state or a tag catalog.
//!
//! Every size reads the game's own flag files through the recording's content
//! layers, so a mod's flags and its cosmetic or ideology variants win exactly as
//! they did in the game. Files decode as the engine's `DecodeFlagTga` does, at
//! the cell sizes the content's defines give. `Shield` also applies the sprite
//! the recorded top bar's `player_flag` button names.

use super::{LoadContext, Product, validate_tag};
use crate::{Source, content, invalid};
use installed_content::{ResolveOptions, content::ResolvedContent, flags::FlagSet, image::Rgba8Image};
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
    /// `gfx/flags`, 82x52.
    #[default]
    Full,
    /// `gfx/flags/medium`, 41x26, as the game's lists draw flags.
    Medium,
    /// `gfx/flags/small`, 10x7.
    Small,
    /// The large flag as the game's top bar draws it through `GFX_shield_medium`.
    Shield,
}

impl CountryFlagSize {
    /// Flag sets searched in order; a medium flag a mod omits falls back to the
    /// same country's large flag rather than to no flag.
    fn sets(self) -> &'static [FlagSet] {
        match self {
            Self::Full | Self::Shield => &[FlagSet::Normal],
            Self::Medium => &[FlagSet::Medium, FlagSet::Normal],
            Self::Small => &[FlagSet::Small],
        }
    }
}

/// The engine finds the top bar's flag by these fixed names; the sprite the
/// button draws is content, read from the file.
const TOPBAR_GUI: &str = "interface/topbar.gui";
const PLAYER_FLAG: &str = "player_flag";
/// Decoded hours kept per source, so a picker and a header following playback
/// do not evict each other.
const RECENT_HOURS: usize = 4;
/// Longest side of any flag the product returns.
const MAX_FLAG_EDGE: u32 = 128;

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
    /// Straight-alpha RGBA bytes, at most 128 pixels on the longer side.
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
        let state = source.state::<FlagState>();
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        match state
            .contexts
            .iter()
            .position(|cached| cached.matches(&content.options, params))
        {
            Some(0) => {}
            Some(index) => {
                let recent = state.contexts.remove(index);
                state.contexts.insert(0, recent);
            }
            None => {
                let resolved = source.content(&content.options)?;
                // The cells are content facts: kept across hours, reread with new content.
                let cells = match state
                    .contexts
                    .iter()
                    .find(|cached| Arc::ptr_eq(&cached.content, &resolved))
                {
                    Some(cached) => cached.cells,
                    None => {
                        let defines = installed_content::defines::read(&resolved.layers)?;
                        [
                            FlagSet::Normal.cell(&defines)?,
                            FlagSet::Medium.cell(&defines)?,
                            FlagSet::Small.cell(&defines)?,
                        ]
                    }
                };
                let loaded = FlagContext::load(
                    source.replay(),
                    resolved,
                    cells,
                    content.options.clone(),
                    content.allow_unverified,
                    params,
                    cancelled,
                )?;
                state.contexts.insert(0, loaded);
                state.contexts.truncate(RECENT_HOURS);
            }
        }
        let shield = if params.size == CountryFlagSize::Shield {
            let current = state
                .shield
                .as_ref()
                .filter(|(owner, _)| Arc::ptr_eq(owner, &state.contexts[0].content))
                .map(|(_, style)| Arc::clone(style));
            Some(match current {
                Some(style) => style,
                None => {
                    let style = Arc::new(ShieldStyle::load(&state.contexts[0].content)?);
                    state.shield =
                        Some((Arc::clone(&state.contexts[0].content), Arc::clone(&style)));
                    style
                }
            })
        } else {
            None
        };
        state.contexts[0].reply(params, shield.as_deref(), cancelled)
    }
}

#[derive(Default)]
struct FlagState {
    /// Most recently used first.
    contexts: Vec<FlagContext>,
    /// The shield read from the content it belongs to; new content rereads it.
    shield: Option<(Arc<ResolvedContent>, Arc<ShieldStyle>)>,
}

/// The top bar's `player_flag` sprite as the recorded content declares it (a
/// `maskedShieldType`, `GFX_shield_medium` in vanilla): `textureFile2` supplies
/// the alpha and `textureFile1` is drawn over the flag.
struct ShieldStyle {
    mask: Rgba8Image,
    overlay: Option<Rgba8Image>,
}

impl ShieldStyle {
    fn load(content: &ResolvedContent) -> io::Result<Self> {
        let topbar = installed_content::gui::read(&content.layers, TOPBAR_GUI)?;
        let name = installed_content::gui::find(&topbar, "buttonType", PLAYER_FLAG)
            .and_then(installed_content::gui::sprite)
            .ok_or_else(|| {
                invalid(format!(
                    "{TOPBAR_GUI} declares no {PLAYER_FLAG} button with a sprite"
                ))
            })?;
        let sprites = installed_content::gfx::sprites(&content.layers)?;
        let sprite = sprites
            .get(name)
            .ok_or_else(|| invalid(format!("{name} is not declared in the recorded content")))?;
        let texture = |key: &str| {
            sprite
                .items
                .iter()
                .rev()
                .find(|item| {
                    item.key
                        .as_deref()
                        .is_some_and(|name| name.eq_ignore_ascii_case(key.as_bytes()))
                })
                .and_then(|item| item.value.scalar_str())
                .map(asset_path)
        };
        let mask = texture("textureFile2")
            .ok_or_else(|| invalid(format!("{name} declares no mask texture")))?;
        Ok(Self {
            mask: interface_texture(&content.layers, &mask)?,
            overlay: texture("textureFile1")
                .map(|path| interface_texture(&content.layers, &path))
                .transpose()?,
        })
    }
}

/// Collapses doubled separators and backslashes in an authored texture path.
fn asset_path(raw: &str) -> String {
    raw.split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

fn interface_texture(layers: &installed_content::ResolvedLayers, path: &str) -> io::Result<Rgba8Image> {
    let bytes = layers.read_uncovered(path)?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(invalid("Flag interface texture exceeds its input budget"));
    }
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".dds") {
        if bytes.len() < 128
            || u32::from_le_bytes(bytes[12..16].try_into().unwrap()) > 1024
            || u32::from_le_bytes(bytes[16..20].try_into().unwrap()) > 1024
        {
            return Err(invalid(
                "Flag interface texture dimensions exceed their budget",
            ));
        }
        installed_content::image::rgba::dds_base(&installed_content::image::dds::decode(&bytes)?)
    } else if lower.ends_with(".tga") {
        if bytes.len() < 18
            || u16::from_le_bytes([bytes[12], bytes[13]]) > 1024
            || u16::from_le_bytes([bytes[14], bytes[15]]) > 1024
        {
            return Err(invalid(
                "Flag interface texture dimensions exceed their budget",
            ));
        }
        installed_content::image::tga::decode(&bytes)
    } else {
        Err(invalid("Unsupported flag interface texture format"))
    }
}

/// The extent `width` x `height` scaled so its longer side is at most `edge`.
fn bounded(width: u32, height: u32, edge: u32) -> (u32, u32) {
    let scale = (width.max(height) as f64 / edge as f64).max(1.0);
    (
        (width as f64 / scale).round().max(1.0) as u32,
        (height as f64 / scale).round().max(1.0) as u32,
    )
}

/// Point-samples `image` across a `width` x `height` grid.
fn sample(image: &Rgba8Image, x: u32, y: u32, width: u32, height: u32) -> [u8; 4] {
    let at = (((y * image.height / height) * image.width + x * image.width / width) * 4) as usize;
    image.pixels[at..at + 4].try_into().unwrap()
}

fn resize(image: &Rgba8Image, width: u32, height: u32) -> Rgba8Image {
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            pixels.extend_from_slice(&sample(image, x, y, width, height));
        }
    }
    Rgba8Image {
        width,
        height,
        pixels,
    }
}

/// `maskedflag.shader` over the sprite's quad: the flag's colour with the
/// mask's alpha, then the authored overlay blended SRC_ALPHA, INV_SRC_ALPHA.
/// The result is straight alpha at the mask's extent.
fn shield(flag: &Rgba8Image, mask: &Rgba8Image, overlay: Option<&Rgba8Image>) -> Rgba8Image {
    let (width, height) = bounded(mask.width, mask.height, MAX_FLAG_EDGE);
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let base = sample(flag, x, y, width, height);
            let base_alpha = u32::from(sample(mask, x, y, width, height)[3]);
            let Some(overlay) = overlay else {
                pixels.extend_from_slice(&[base[0], base[1], base[2], base_alpha as u8]);
                continue;
            };
            let top = sample(overlay, x, y, width, height);
            let top_alpha = u32::from(top[3]);
            // Alpha of `top` over `base`, scaled by 255 * 255.
            let alpha = top_alpha * 255 + base_alpha * (255 - top_alpha);
            let mut pixel = [0_u8; 4];
            for channel in 0..3 {
                let weighted = u32::from(top[channel]) * top_alpha * 255
                    + u32::from(base[channel]) * base_alpha * (255 - top_alpha)
                    + alpha / 2;
                pixel[channel] = weighted.checked_div(alpha).unwrap_or(0) as u8;
            }
            pixel[3] = ((alpha + 127) / 255) as u8;
            pixels.extend_from_slice(&pixel);
        }
    }
    Rgba8Image {
        width,
        height,
        pixels,
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
    /// The normal, medium and small cells from the content's defines.
    cells: [[u32; 2]; 3],
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
        cells: [[u32; 2]; 3],
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
            cells,
            choices,
            branch_id: request.branch_id.clone(),
            hour: request.hour,
            warnings,
        })
    }

    fn reply(
        &self,
        request: &CountryFlagParams,
        shield_style: Option<&ShieldStyle>,
        cancelled: &AtomicBool,
    ) -> io::Result<CountryFlagReply> {
        let mut flags = Vec::new();
        let mut warnings = self.warnings.clone();
        for tag in &request.tags {
            if cancelled.load(Ordering::Acquire) {
                return Err(invalid("Flag request cancelled"));
            }
            let Some(choice) = self.choices.iter().find(|choice| &choice.tag == tag) else {
                warnings.push(format!("{tag}: country identity unavailable at this hour"));
                continue;
            };
            let mut found = None;
            'search: for &set in request.size.sets() {
                let cell = self.cells[match set {
                    FlagSet::Normal => 0,
                    FlagSet::Medium => 1,
                    FlagSet::Small => 2,
                }];
                for key in &choice.candidates {
                    if key.is_empty()
                        || key.len() > 256
                        || !key
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                    {
                        return Err(invalid("Invalid recorded flag asset name"));
                    }
                    let asset = format!("{}{key}.tga", set.directory());
                    let bytes = match self.content.layers.read_uncovered(&asset) {
                        Ok(bytes) => bytes,
                        Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                        Err(error) => return Err(error),
                    };
                    // As `LoadFirstDecodableCountryFlag` (`14122a730`), a file the
                    // engine's decoder refuses falls through to the next candidate.
                    let Some(image) = installed_content::flags::decode(&bytes, cell) else {
                        continue;
                    };
                    let image = match shield_style {
                        Some(style) => shield(&image, &style.mask, style.overlay.as_ref()),
                        None => {
                            let (width, height) = bounded(image.width, image.height, MAX_FLAG_EDGE);
                            resize(&image, width, height)
                        }
                    };
                    found = Some(CountryFlagImage {
                        tag: tag.clone(),
                        asset,
                        width: image.width,
                        height: image.height,
                        pixels: image.pixels,
                    });
                    break 'search;
                }
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
    fn shield_keeps_flag_colour_takes_mask_alpha_and_blends_the_overlay() {
        let image = |width, height, pixels: Vec<u8>| Rgba8Image {
            width,
            height,
            pixels,
        };
        // A translucent authored flag pixel stays opaque: the shader takes alpha from the mask.
        let flag = image(2, 1, vec![200, 0, 0, 90, 0, 0, 200, 255]);
        let mask = image(2, 1, vec![255, 255, 255, 255, 0, 0, 0, 0]);
        assert_eq!(
            shield(&flag, &mask, None).pixels,
            [200, 0, 0, 255, 0, 0, 200, 0]
        );

        let overlay = image(1, 1, vec![0, 0, 0, 128]);
        let drawn = shield(&flag, &mask, Some(&overlay));
        assert_eq!((drawn.width, drawn.height), (2, 1));
        // Half-black over opaque red; over a fully masked pixel only the overlay remains.
        assert_eq!(&drawn.pixels[..4], [100, 0, 0, 255]);
        assert_eq!(&drawn.pixels[4..], [0, 0, 0, 128]);

        // The sprite's extent wins over the flag's, bounded to the reply budget.
        let large = image(300, 150, vec![255; 300 * 150 * 4]);
        assert_eq!(
            (
                shield(&flag, &large, None).width,
                shield(&flag, &large, None).height
            ),
            (128, 64)
        );
    }

    #[test]
    fn sizes_search_the_game_flag_sets_in_order() {
        assert_eq!(
            CountryFlagSize::Medium.sets(),
            [FlagSet::Medium, FlagSet::Normal]
        );
        assert_eq!(CountryFlagSize::Shield.sets(), [FlagSet::Normal]);
        assert_eq!(
            asset_path("gfx//interface\\flag_overlay.dds"),
            "gfx/interface/flag_overlay.dds"
        );
        assert_eq!(bounded(82, 52, 128), (82, 52));
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
        for (name, size) in [
            ("medium", CountryFlagSize::Medium),
            ("shield", CountryFlagSize::Shield),
        ] {
            value["size"] = serde_json::json!(name);
            let params: CountryFlagParams = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(params.size, size);
        }
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
