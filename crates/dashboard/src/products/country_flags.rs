//! Country flags from the recorded content stack, never launcher state or a tag catalog.
//!
//! Every size reads the game's own flag files through the recording's content
//! layers, so a mod's flags and its cosmetic or ideology variants win exactly as
//! they did in the game. Files decode as the engine's `DecodeFlagTga` does, at
//! the cell sizes the content's defines give. `Shield` also applies the sprite
//! the recorded top bar's `player_flag` button names.

use super::{LoadContext, Product, recorded_rows, validate_tag};
use crate::{Source, catalogue::Catalogue, content, invalid};
use installed_content::{ResolveOptions, content::ResolvedContent, flags::FlagSet, image::Rgba8Image};
use replay::{
    container::{Replay, SeekResult},
    frame::DecodedFrame,
    rows::tables,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
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
    pub(crate) fn sets(self) -> &'static [FlagSet] {
        match self {
            Self::Full | Self::Shield => &[FlagSet::Normal],
            Self::Medium => &[FlagSet::Medium, FlagSet::Normal],
            Self::Small => &[FlagSet::Small],
        }
    }
}

/// The window the engine opens as the top bar: `topbar` (`142934CD8`), which
/// `technology_canvas_owner::constructor@140B52390` passes to
/// `CTopBar::CTopBar@14187C8D0` (kept at `+0x48`), and
/// `hud_topbar_controller_layout::setup@14188BA30` opens through
/// `CGui::container_factory@142246270`.
const TOP_BAR: &str = "topbar";
/// The flag button setup then asks that window for
/// (`CContainerWindow::FindButton@1422A4A20`): `player_flag` at `1429FA990`,
/// also used by `CTopBar::UpdateContent@14188EC80`. The sprite it draws is
/// content, read from the top bar's own declaration wherever it is.
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
    const CONTENT_DEPENDENT: bool = true;
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
                    None => flag_cells(&resolved.layers)?,
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
        let mut load_shield = || -> io::Result<Arc<ShieldStyle>> {
            let catalogue = crate::catalogue::catalogue(source, content)?;
            let decode = *catalogue.gui_decode.as_ref().map_err(|why| {
                invalid(format!(
                    "the top bar's {PLAYER_FLAG} shield artwork is unavailable: {why}"
                ))
            })?;
            let current = state
                .shield
                .as_ref()
                .filter(|(owner, setting, _)| {
                    Arc::ptr_eq(owner, &state.contexts[0].content) && *setting == decode
                })
                .map(|(_, _, style)| style.clone());
            let style = match current {
                Some(style) => style,
                None => {
                    let style = ShieldStyle::load(&catalogue)
                        .map(Arc::new)
                        .map_err(|error| error.to_string());
                    state.shield = Some((
                        Arc::clone(&state.contexts[0].content),
                        decode,
                        style.clone(),
                    ));
                    style
                }
            };
            style.map_err(invalid)
        };
        let shield = (params.size == CountryFlagSize::Shield)
            .then(|| load_shield().map_err(|error| error.to_string()));
        state.contexts[0].reply(
            params,
            shield
                .as_ref()
                .map(|style| style.as_ref().map(Arc::as_ref).map_err(String::as_str)),
            cancelled,
        )
    }
}

#[derive(Default)]
struct FlagState {
    /// Most recently used first.
    contexts: Vec<FlagContext>,
    /// The shield read from the content it belongs to; new content rereads it.
    shield: Option<CachedShield>,
}

/// The shield as last loaded for (content, decode), a refusal included, so a
/// refused shield is not decoded again on every request.
type CachedShield = (
    Arc<ResolvedContent>,
    installed_content::image::native::Decode,
    Result<Arc<ShieldStyle>, String>,
);

/// The top bar's `player_flag` sprite as the recorded content declares it (a
/// `maskedShieldType`, `GFX_shield_medium` in vanilla): `textureFile2` supplies
/// the alpha and `textureFile1` is drawn over the flag.
pub(crate) struct ShieldStyle {
    mask: Rgba8Image,
    overlay: Option<Rgba8Image>,
    pub(crate) warnings: Vec<String>,
}

impl ShieldStyle {
    pub(crate) fn load(catalogue: &Catalogue) -> io::Result<Self> {
        let content = &catalogue.content;
        let decode = *catalogue.gui_decode.as_ref().map_err(|why| {
            invalid(format!(
                "the top bar's {PLAYER_FLAG} shield artwork is unavailable: {why}"
            ))
        })?;
        let mut warnings = Vec::new();
        let name = installed_content::gui::native::window_button_sprite(
            &catalogue.guis,
            TOP_BAR,
            PLAYER_FLAG,
            &mut warnings,
        )?;
        let sprite = crate::catalogue::created_gui_sprite(&catalogue.sprites, &name)?;
        let installed_content::gui::native::sprites::SpriteKind::MaskedShield { shield, .. } =
            &sprite.facts.kind
        else {
            return Err(invalid(format!("{name} is not a maskedShieldType")));
        };
        if shield.texture2.is_empty() {
            return Err(invalid(format!("{name} declares no mask texture")));
        }
        Ok(Self {
            mask: crate::catalogue::gui_texture(&content.layers, &shield.texture2, decode)?,
            overlay: if shield.texture1.is_empty() {
                None
            } else {
                Some(crate::catalogue::gui_texture(
                    &content.layers,
                    &shield.texture1,
                    decode,
                )?)
            },
            warnings,
        })
    }
}

/// Whether a recorded flag key names a file this reader can resolve inside a
/// flag folder. The engine's path is the folder, the key and `.tga` joined as
/// `variants/red` names `gfx/flags/variants/red.tga`. A key is accepted when it
/// is bounded and every `/`-separated part is non-empty, not `.` or `..`, and
/// free of backslashes, NUL and drive colons: so it stays below the folder.
/// Tags are otherwise opaque (spaces, lower case and other bytes are the
/// country's own).
fn safe_flag_key(key: &str) -> bool {
    key.len() <= 256
        && key.split('/').all(|part| {
            !part.is_empty() && part != "." && part != ".." && !part.contains(['\\', '\0', ':'])
        })
}

/// The flag the engine draws for one country from its candidate keys in order.
#[derive(Debug)]
pub(crate) enum FlagSearch {
    Found {
        asset: String,
        image: Arc<Rgba8Image>,
    },
    /// A candidate this reader cannot resolve: the engine would have tried it
    /// before the rest, so no later candidate may stand in for it.
    Unresolvable,
    Absent,
}

pub(crate) fn flag_cells(layers: &installed_content::ResolvedLayers) -> io::Result<[[u32; 2]; 3]> {
    let defines = installed_content::defines::read(layers)?;
    Ok([
        FlagSet::Normal.cell(&defines)?,
        FlagSet::Medium.cell(&defines)?,
        FlagSet::Small.cell(&defines)?,
    ])
}

#[derive(Clone)]
enum FlagFile {
    Missing,
    Refused(installed_content::flags::DecodeFailure),
    Image(Arc<Rgba8Image>),
}

/// One decoding per file/set, shared across the country's key ladders.
#[derive(Default)]
pub(crate) struct FlagCache {
    files: BTreeMap<(usize, String), FlagFile>,
    reported: BTreeSet<(usize, String)>,
    substitutions: BTreeSet<String>,
    unresolvable: BTreeSet<String>,
}

/// Searches `sets` in order, and in each the candidates in order, as
/// `LoadFirstDecodableCountryFlag` (`14122A730`) does: a missing file or one
/// the engine's decoder refuses falls through to the next candidate. A
/// candidate [`safe_flag_key`] refuses stops the search, because falling past
/// it would draw a flag the game does not.
pub(crate) fn find_flag(
    layers: &installed_content::ResolvedLayers,
    sets: &[FlagSet],
    cells: &[[u32; 2]; 3],
    candidates: &[String],
    cache: &mut FlagCache,
) -> io::Result<(FlagSearch, Vec<crate::content_check::Finding>)> {
    use crate::content_check::{Finding, Kind};
    let mut findings = Vec::new();
    for &set in sets {
        let set_index = match set {
            FlagSet::Normal => 0,
            FlagSet::Medium => 1,
            FlagSet::Small => 2,
        };
        let cell = cells[set_index];
        for key in candidates {
            if !safe_flag_key(key) {
                if cache.unresolvable.insert(key.clone()) {
                    findings.push(Finding::new(Kind::Refused, key,
                        format!("flag name {key:?} cannot be resolved inside the flag folder, so the country's flag is unavailable")));
                }
                return Ok((FlagSearch::Unresolvable, findings));
            }
            let asset = format!("{}{key}.tga", set.directory());
            let cache_key = (set_index, asset.clone());
            if !cache.files.contains_key(&cache_key) {
                let file = match layers.read_uncovered(&asset) {
                    Ok(bytes) => match installed_content::flags::decode_checked(&bytes, cell) {
                        Ok(image) => FlagFile::Image(Arc::new(image)),
                        Err(reason) => FlagFile::Refused(reason),
                    },
                    Err(error) if error.kind() == io::ErrorKind::NotFound => FlagFile::Missing,
                    Err(error) => return Err(error),
                };
                cache.files.insert(cache_key.clone(), file);
            }
            match &cache.files[&cache_key] {
                FlagFile::Missing => {}
                FlagFile::Refused(reason) => {
                    if !cache.reported.insert(cache_key.clone()) {
                        continue;
                    }
                    // DecodeFlagTga 14122AB00 rejects these headers, and
                    // LoadFirstDecodableCountryFlag 14122A730 skips them.
                    // The port's stricter truncated-read refusal differs.
                    let mut finding = Finding::new(
                        match reason {
                            installed_content::flags::DecodeFailure::EngineRefused => Kind::Content,
                            installed_content::flags::DecodeFailure::Truncated => Kind::Refused,
                        },
                        key,
                        match reason {
                            installed_content::flags::DecodeFailure::EngineRefused => format!(
                                "flag decoder refused {asset} at cell {}x{}; this file was skipped",
                                cell[0], cell[1]
                            ),
                            installed_content::flags::DecodeFailure::Truncated => format!(
                                "flag decoder cannot safely read {asset} at cell {}x{}; the port refuses this input but the engine does not check read status",
                                cell[0], cell[1]
                            ),
                        },
                    );
                    finding.file = Some(asset.clone());
                    findings.push(finding);
                }
                FlagFile::Image(image) => {
                    if sets.first() == Some(&FlagSet::Medium)
                        && set == FlagSet::Normal
                        && cache.substitutions.insert(asset.clone())
                    {
                        // No traced native medium-to-normal substitution: actionable.
                        let mut finding = Finding::new(
                            Kind::Fallback,
                            key,
                            format!(
                                "medium flag is missing or undecodable; normal flag {asset} is shown"
                            ),
                        );
                        finding.file = Some(asset.clone());
                        findings.push(finding);
                    }
                    return Ok((
                        FlagSearch::Found {
                            asset,
                            image: image.clone(),
                        },
                        findings,
                    ));
                }
            }
        }
    }
    // Product omission has no cited native display parity.
    findings.push(Finding::new(
        Kind::Fallback,
        candidates.first().cloned().unwrap_or_default(),
        "no decodable flag in the requested sets; flag artwork is unavailable",
    ));
    Ok((FlagSearch::Absent, findings))
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

pub struct FlagChoice {
    pub raw: u32,
    pub tag: String,
    pub candidates: Vec<String>,
}

/// Flag asset candidates per country at one hour, shared with the combat UI.
pub fn choices(frame: &DecodedFrame) -> io::Result<Vec<FlagChoice>> {
    let links: BTreeMap<_, _> = recorded_rows::<tables::CountryLinks>(frame)?
        .iter()
        .map(|r| (r.id, r.canonical_slot))
        .collect();
    let countries: BTreeMap<_, _> = recorded_rows::<tables::Countries>(frame)?
        .iter()
        .map(|r| {
            (
                r.canonical_slot,
                String::from_utf8_lossy(&r.tag).into_owned(),
            )
        })
        .collect();
    let parties: BTreeMap<_, _> = recorded_rows::<tables::PoliticalParties>(frame)?
        .iter()
        .map(|r| {
            (
                (r.country, r.id),
                String::from_utf8_lossy(&r.token).into_owned(),
            )
        })
        .collect();
    let politics: BTreeMap<_, _> = recorded_rows::<tables::Politics>(frame)?
        .iter()
        .filter_map(|r| Some((r.country, parties.get(&(r.country, r.ruling_party?))?)))
        .collect();
    let appearances: BTreeMap<_, _> = recorded_rows::<tables::CountryAppearance>(frame)?
        .iter()
        .map(|r| {
            (
                r.country,
                (
                    String::from_utf8_lossy(&r.cosmetic_tag).into_owned(),
                    String::from_utf8_lossy(&r.name_tag).into_owned(),
                ),
            )
        })
        .collect();
    Ok(links
        .into_iter()
        .filter_map(|(raw, slot)| {
            let tag = countries.get(&slot)?;
            let (cosmetic, original) = appearances
                .get(&raw)
                .map(|(cosmetic, original)| (cosmetic.as_str(), original.as_str()))
                .unwrap_or_default();
            let candidates = installed_content::country_names::flag_keys(
                tag,
                original,
                cosmetic,
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

pub struct FlagContext {
    options: ResolveOptions,
    content: Arc<ResolvedContent>,
    /// The normal, medium and small cells from the content's defines.
    cells: [[u32; 2]; 3],
    /// The first choice per tag, in raw country id order.
    choices: HashMap<String, FlagChoice>,
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
        let mut by_tag = HashMap::new();
        for choice in choices(&frame)? {
            by_tag.entry(choice.tag.clone()).or_insert(choice);
        }
        Ok(Self {
            options,
            content,
            cells,
            choices: by_tag,
            branch_id: request.branch_id.clone(),
            hour: request.hour,
            warnings,
        })
    }

    fn reply(
        &self,
        request: &CountryFlagParams,
        shield_style: Option<Result<&ShieldStyle, &str>>,
        cancelled: &AtomicBool,
    ) -> io::Result<CountryFlagReply> {
        let mut flags = Vec::new();
        let mut warnings = self.warnings.clone();
        let shield_style = match shield_style {
            Some(Ok(style)) => Some(style),
            Some(Err(reason)) => {
                warnings.push(format!("Shield artwork unavailable: {reason}"));
                return Ok(CountryFlagReply {
                    branch_id: request.branch_id.clone(),
                    hour: request.hour,
                    size: request.size,
                    flags,
                    warnings,
                });
            }
            None => None,
        };
        let mut cache = FlagCache::default();
        if let Some(style) = shield_style {
            warnings.extend(style.warnings.iter().cloned());
        }
        for tag in &request.tags {
            if cancelled.load(Ordering::Acquire) {
                return Err(invalid("Flag request cancelled"));
            }
            let Some(choice) = self.choices.get(tag) else {
                warnings.push(format!("{tag}: country identity unavailable at this hour"));
                continue;
            };
            // A flag key is the recorded tag (any bytes but NUL, spaces
            // included) with a cosmetic tag or party token; one this reader
            // cannot resolve costs only this country's flag.
            let (found, findings) = find_flag(
                &self.content.layers,
                request.size.sets(),
                &self.cells,
                &choice.candidates,
                &mut cache,
            )?;
            warnings.extend(
                findings
                    .into_iter()
                    // Substitution and engine-skipped files stay corpus-only.
                    // Refusals differ from a file the engine could draw.
                    .filter(|finding| finding.kind == crate::content_check::Kind::Refused)
                    .map(|finding| format!("{tag}: {}", finding.message)),
            );
            match found {
                FlagSearch::Found { asset, image } => {
                    let image = match shield_style {
                        Some(style) => shield(&image, &style.mask, style.overlay.as_ref()),
                        None => {
                            let (width, height) = bounded(image.width, image.height, MAX_FLAG_EDGE);
                            resize(&image, width, height)
                        }
                    };
                    flags.push(CountryFlagImage {
                        tag: tag.clone(),
                        asset,
                        width: image.width,
                        height: image.height,
                        pixels: image.pixels,
                    });
                }
                FlagSearch::Unresolvable => {}
                FlagSearch::Absent => warnings.push(format!(
                    "{tag}: flag artwork unavailable in the recorded content"
                )),
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
    fn unresolvable_flag_keys_report_once_across_sets_and_groups() {
        let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[]);
        let mut cache = FlagCache::default();
        for (index, set) in [FlagSet::Normal, FlagSet::Medium, FlagSet::Small]
            .into_iter()
            .enumerate()
        {
            let (found, findings) = find_flag(
                &catalogue.content.layers,
                &[set],
                &[[1, 1]; 3],
                &["../mod_country".into(), "safe_fallback".into()],
                &mut cache,
            )
            .unwrap();
            assert!(matches!(found, FlagSearch::Unresolvable));
            assert_eq!(findings.len(), usize::from(index == 0));
        }
    }

    #[test]
    fn live_flag_reply_warns_only_for_unreproduced_draws() {
        let image = vec![
            0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 32, 0x28, 30, 20, 10, 255,
        ];
        let mut skipped = image.clone();
        skipped[2] = 10;
        let mut truncated = image.clone();
        truncated.pop();
        let (catalogue, _folders) = crate::catalogue::texture_tests::texture_catalogue(
            &[],
            &[
                ("gfx/flags/mod_normal.tga", image.clone()),
                ("gfx/flags/medium/mod_good.tga", image),
                ("gfx/flags/medium/mod_skipped.tga", skipped),
                ("gfx/flags/medium/mod_truncated.tga", truncated),
            ],
        );
        for (candidates, asset, reason) in [
            (vec!["mod_normal"], Some("gfx/flags/mod_normal.tga"), None),
            (
                vec!["mod_skipped", "mod_good"],
                Some("gfx/flags/medium/mod_good.tga"),
                None,
            ),
            (
                vec!["mod_truncated", "mod_good"],
                Some("gfx/flags/medium/mod_good.tga"),
                Some("engine does not check read status"),
            ),
            (
                vec!["mod_absent"],
                None,
                Some("mod tag: flag artwork unavailable in the recorded content"),
            ),
        ] {
            let context = FlagContext {
                options: ResolveOptions {
                    game_dir: "G".into(),
                    user_dir: "U".into(),
                    cache_dir: "C".into(),
                },
                content: Arc::clone(&catalogue.content),
                cells: [[1, 1]; 3],
                choices: HashMap::from([(
                    "mod tag".into(),
                    FlagChoice {
                        raw: 0,
                        tag: "mod tag".into(),
                        candidates: candidates.into_iter().map(str::to_owned).collect(),
                    },
                )]),
                branch_id: "1".into(),
                hour: 0,
                warnings: Vec::new(),
            };
            let request = CountryFlagParams {
                branch_id: "1".into(),
                hour: 0,
                tags: vec!["mod tag".into()],
                size: CountryFlagSize::Medium,
            };
            let reply = context
                .reply(&request, None, &AtomicBool::new(false))
                .unwrap();
            assert_eq!(reply.flags.first().map(|flag| flag.asset.as_str()), asset);
            match reason {
                Some(reason) => assert_eq!(
                    reply
                        .warnings
                        .iter()
                        .filter(|warning| warning.contains(reason))
                        .count(),
                    1,
                    "{:?}",
                    reply.warnings
                ),
                None => assert!(reply.warnings.is_empty(), "{:?}", reply.warnings),
            }
        }
    }

    #[test]
    fn unknown_decode_settings_refuse_the_shield_by_name() {
        let (mut catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[]);
        catalogue.gui_decode = Err("settings.txt is unavailable".into());
        let why = ShieldStyle::load(&catalogue).err().unwrap().to_string();
        assert!(why.contains("player_flag shield artwork"), "{why}");
        assert!(why.contains("settings.txt"), "{why}");
        assert_shield_warning(&catalogue, &why);
    }

    fn assert_shield_warning(catalogue: &crate::catalogue::Catalogue, reason: &str) {
        let context = FlagContext {
            options: ResolveOptions {
                game_dir: "G".into(),
                user_dir: "U".into(),
                cache_dir: "C".into(),
            },
            content: Arc::clone(&catalogue.content),
            cells: [[1, 1]; 3],
            choices: HashMap::new(),
            branch_id: "1".into(),
            hour: 0,
            warnings: vec!["standing warning".into()],
        };
        let request = CountryFlagParams {
            branch_id: "1".into(),
            hour: 0,
            tags: vec!["MOD tag".into()],
            size: CountryFlagSize::Shield,
        };
        let reply = context
            .reply(&request, Some(Err(reason)), &AtomicBool::new(false))
            .unwrap();
        assert!(reply.flags.is_empty());
        assert_eq!(
            reply.warnings,
            [
                "standing warning",
                &format!("Shield artwork unavailable: {reason}")
            ]
        );
    }

    #[test]
    fn shield_png_padding_and_every_refusal_use_the_shared_gui_path() {
        use crate::catalogue::texture_tests::{png_bytes, refusal_textures, texture_catalogue};
        let mask = Rgba8Image {
            width: 3,
            height: 2,
            pixels: vec![200; 24],
        };
        let refusals = refusal_textures();
        let mut textures: Vec<_> = refusals
            .iter()
            .filter(|(_, bytes, _)| !bytes.is_empty())
            .map(|(path, bytes, _)| (*path, bytes.clone()))
            .collect();
        textures.extend([
            ("gfx/mod/mask.png", png_bytes(&mask)),
            ("gfx/mod/overlay.png", png_bytes(&mask)),
        ]);
        let (mut catalogue, _folders) = texture_catalogue(
            &[
                (
                    "interface/topbar.gui",
                    "guiTypes = { containerWindowType = { name = topbar buttonType = { name = player_flag spriteType = GFX_mod_shield } } }",
                ),
                (
                    "interface/art.gfx",
                    "spriteTypes = { maskedShieldType = { name = GFX_mod_shield textureFile1 = gfx/mod/overlay.png textureFile2 = gfx/mod/mask.png } }",
                ),
            ],
            &textures,
        );
        catalogue.gui_decode = Ok(installed_content::image::native::Decode {
            force_pow2_textures: true,
        });
        let loaded = ShieldStyle::load(&catalogue).unwrap();
        assert_eq!((loaded.mask.width, loaded.mask.height), (4, 2));
        assert_eq!(loaded.overlay.unwrap().width, 4);
        for (path, _, expected) in refusals {
            let facts = installed_content::gui::native::sprites::read_sprites(format!(
                "spriteTypes = {{ maskedShieldType = {{ name = GFX_mod_shield textureFile2 = \"{path}\" }} }}"
            ).as_bytes());
            catalogue.sprites =
                installed_content::gfx::SpriteSet::new(&[("interface/art.gfx".into(), facts)]);
            let why = ShieldStyle::load(&catalogue).err().unwrap().to_string();
            assert!(why.contains(expected), "{path}: {why}");
            assert_shield_warning(&catalogue, &why);
        }
    }

    #[test]
    fn sizes_search_the_game_flag_sets_in_order() {
        assert_eq!(
            CountryFlagSize::Medium.sets(),
            [FlagSet::Medium, FlagSet::Normal]
        );
        assert_eq!(CountryFlagSize::Shield.sets(), [FlagSet::Normal]);
        assert_eq!(bounded(82, 52, 128), (82, 52));
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
        // Request tags are lookup keys; asset names come from recorded rows.
        params.tags = vec!["A\0B".into()];
        assert!(CountryFlags::validate(&params).is_err());
        params.tags = vec![String::new()];
        assert!(CountryFlags::validate(&params).is_err());
        params.tags = vec!["Z01".into(); 33];
        assert!(CountryFlags::validate(&params).is_err());
    }
}
