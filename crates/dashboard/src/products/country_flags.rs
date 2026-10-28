//! Country flags from the recorded content stack, never launcher state or a tag catalog.
//!
//! Every size reads the game's own flag files through the recording's content
//! layers, so a mod's flags and its cosmetic or ideology variants win exactly as
//! they did in the game. Files decode as the engine's `DecodeFlagTga` does, at
//! the cell sizes the content's defines give. `Shield` draws the flag as the
//! recorded top bar's `player_flag` button draws it, through its masked shield;
//! `Panel` is app styling: the same draw with the overlay's edge shading evened
//! out, for flags drawn inside the dashboard's panels.

use super::{LoadContext, Product, validate_tag};
pub use crate::flag_choices::{FlagChoice, choices, ruling_groups};
use crate::{Source, catalogue::Catalogue, content, invalid};
use installed_content_core::{ResolveOptions, content::ResolvedContent, flags::FlagSet};
use installed_content_image::Rgba8Image;
use replay::{container::Replay, container::SeekResult};
use replay_data::rows::TableId;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{
        Arc, Mutex, OnceLock,
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
    /// The flag as the game's top bar draws it through its masked shield
    /// (`GFX_shield_medium` in vanilla).
    Shield,
    /// App styling, not a game rendering: the top bar's shield drawn with its
    /// overlay's broad edge shading evened out to the overlay's centre, keeping
    /// its texture, for flags drawn inside panels.
    Panel,
}

impl CountryFlagSize {
    /// Flag sets searched in order; a medium flag a mod omits falls back to the
    /// same country's large flag rather than to no flag. A shield's descriptor
    /// chooses its set ([`ShieldStyle::sets`]).
    pub(crate) fn sets(self) -> &'static [FlagSet] {
        match self {
            Self::Full => &[FlagSet::Normal],
            Self::Medium => &[FlagSet::Medium, FlagSet::Normal],
            Self::Small => &[FlagSet::Small],
            Self::Shield | Self::Panel => &[],
        }
    }
}

/// The top bar opens the topbar window from its authored declaration.
const TOP_BAR: &str = "topbar";
/// The flag button uses player_flag. Its sprite is content, read from the
/// top bar's own declaration wherever it is.
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
    /// The flag file drawn, as the content names it.
    pub asset: String,
    /// What the flag shows: its file and a digest of its pixels, so two flags share it only
    /// when they show the same picture (the visual mods' file and the recording's of one path
    /// are two).
    pub key: String,
    pub width: u32,
    pub height: u32,
    /// Straight-alpha RGBA bytes, at most 128 pixels on the longer side.
    pub pixels: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CountryFlagUnavailable {
    pub tag: String,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CountryFlagReply {
    pub branch_id: String,
    pub hour: i32,
    pub size: CountryFlagSize,
    pub flags: Vec<CountryFlagImage>,
    pub unavailable: Vec<CountryFlagUnavailable>,
    pub warnings: Vec<String>,
}

impl Product for CountryFlags {
    const KEY: &'static str = "country-flags";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = CountryFlagParams;
    type Reply = CountryFlagReply;

    fn types() -> Vec<String> {
        vec![
            CountryFlagSize::decl(),
            CountryFlagImage::decl(),
            CountryFlagUnavailable::decl(),
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
        let presentation = content.presentation.identity().clone();
        // The binding's mount, resolved in its cancellable cell before any flag lock is taken.
        let presented = source.presented_for(content)?;
        let shared = source.state::<FlagState>();
        let mut state = shared.lock().unwrap_or_else(|error| error.into_inner());
        match state
            .contexts
            .iter()
            .position(|cached| cached.matches(&content.options, &presentation, params))
        {
            Some(0) => {}
            Some(index) => {
                let recent = state.contexts.remove(index);
                state.contexts.insert(0, recent);
            }
            None => {
                // Flags follow the source's visual mods; recorded facts read the recorded layers.
                let resolved = Arc::clone(&presented.content);
                // The cells and flag files are content facts: kept across hours, reread with
                // new content.
                let cells = state
                    .contexts
                    .iter()
                    .find(|cached| Arc::ptr_eq(&cached.content, &resolved))
                    .map(|cached| {
                        (
                            cached.cells,
                            cached.cell_fallbacks.clone(),
                            Arc::clone(&cached.files),
                        )
                    });
                // Other hours' requests use the cache while this one reads the recording.
                drop(state);
                let (cells, cell_fallbacks, files) = match cells {
                    Some(cells) => cells,
                    None => {
                        let (cells, fallbacks) =
                            installed_content_core::content::observe_overlay_refusals(|| {
                                flag_cells(&resolved.layers)
                            });
                        (cells?, fallbacks, Arc::default())
                    }
                };
                let mut loaded = FlagContext::load(
                    source.replay(),
                    resolved,
                    cells,
                    content.options.clone(),
                    content.allow_unverified,
                    params,
                    cancelled,
                )?;
                loaded.presentation = presented.presentation.clone();
                loaded.cell_fallbacks = cell_fallbacks;
                loaded.files = files;
                state = shared.lock().unwrap_or_else(|error| error.into_inner());
                state.contexts.insert(0, loaded);
                state.contexts.truncate(RECENT_HOURS);
            }
        }
        installed_content_core::content::credit_overlay_refusals(&state.contexts[0].cell_fallbacks);
        let mut load_shield = || -> io::Result<Arc<ShieldStyle>> {
            // The catalogue and the flag context are both of the request's admitted presentation.
            let catalogue = crate::catalogue::catalogue(source, content)?;
            cached_shield(&mut state.shield, &catalogue)
        };
        let shield = matches!(
            params.size,
            CountryFlagSize::Shield | CountryFlagSize::Panel
        )
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

/// The shield `catalogue` supplies, kept in `slot` by that catalogue's content and decode, so a
/// shield is never served for a catalogue (a presentation) other than the one it came from.
fn cached_shield(
    slot: &mut Option<CachedShield>,
    catalogue: &Catalogue,
) -> io::Result<Arc<ShieldStyle>> {
    let decode = *catalogue.gui_decode.as_ref().map_err(|why| {
        invalid(format!(
            "the top bar's {PLAYER_FLAG} shield artwork is unavailable: {why}"
        ))
    })?;
    let current = slot
        .as_ref()
        .filter(|(owner, setting, _)| Arc::ptr_eq(owner, &catalogue.content) && *setting == decode)
        .map(|(_, _, style)| style.get().clone());
    let style = match current {
        Some(style) => style,
        None => {
            let drawn = crate::catalogue::Drawn::new(|| {
                ShieldStyle::load(catalogue)
                    .map(Arc::new)
                    .map_err(|error| error.to_string())
            });
            let style = drawn.get().clone();
            *slot = Some((Arc::clone(&catalogue.content), decode, drawn));
            style
        }
    };
    style.map_err(invalid)
}

#[derive(Default)]
struct FlagState {
    /// Most recently used first.
    contexts: Vec<FlagContext>,
    /// The shield read from the catalogue it belongs to; another catalogue rereads it.
    shield: Option<CachedShield>,
}

/// The shield as last loaded for (the catalogue's content, decode), a refusal included, so a
/// refused shield is not decoded again on every request.
type CachedShield = (
    Arc<ResolvedContent>,
    installed_content_image::native::Decode,
    crate::catalogue::Drawn<Result<Arc<ShieldStyle>, String>>,
);

/// The top bar's `player_flag` sprite as the content declares it (a `maskedShieldType`,
/// `GFX_shield_medium` in vanilla), drawn as the game draws it: the visual mods' when they
/// supply one that loads and draws, else the recording's own, named.
pub(crate) struct ShieldStyle {
    shield: crate::catalogue::MaskedShield,
    /// The shield as `Panel` draws it: its overlay evened once per content stack.
    panel: crate::catalogue::MaskedShield,
    /// When the visual mods' shield cannot load or draw, the recording's layers and flag cells
    /// it and the flags it frames are read from, as the recording draws them. `None` reads the
    /// presentation of the flag context's content.
    recorded: Option<(installed_content_core::ResolvedLayers, [[u32; 2]; 3])>,
    /// The flag files read from `recorded`, kept with this shield.
    recorded_files: Arc<FlagFiles>,
    decode: installed_content_image::native::Decode,
    pub(crate) warnings: Vec<String>,
}

impl ShieldStyle {
    pub(crate) fn load(catalogue: &Catalogue) -> io::Result<Self> {
        let decode = *catalogue.gui_decode.as_ref().map_err(|why| {
            invalid(format!(
                "the top bar's {PLAYER_FLAG} shield artwork is unavailable: {why}"
            ))
        })?;
        let layers = &catalogue.content.layers;
        let mut warnings = Vec::new();
        let (shield, recorded) = if !layers.presentation().shows_visual_mods() {
            (Self::shown(catalogue, decode, &mut warnings)?, None)
        } else {
            // One transaction, as `catalogue::present` draws: the visual mods' whole choice,
            // strictly and with its notes kept to itself, then the recording's whole choice
            // when any stage of theirs fails.
            let (shown, noted) = crate::catalogue::capture_isolated(|| {
                installed_content_core::content::strict(|| Self::shown(catalogue, decode, &mut warnings))
            });
            match shown {
                Ok(shield) => {
                    crate::catalogue::credit(&noted);
                    (shield, None)
                }
                Err(error) => {
                    // The recording's own top bar binding, its declaration, flag set and cells.
                    let recorded = layers.recorded_only();
                    warnings.clear();
                    let guis = installed_content_core::gui::native::read_layers(&recorded)?;
                    let name = installed_content_core::gui::native::window_button_sprite(
                        &guis,
                        TOP_BAR,
                        PLAYER_FLAG,
                        &mut warnings,
                    )?;
                    let sprites = crate::catalogue::recorded_sprites(&recorded);
                    let sprites = (*sprites).as_ref().map_err(|why| invalid(why.clone()))?;
                    let sprite = crate::catalogue::created_gui_sprite(sprites, &name)?;
                    let shield = crate::catalogue::MaskedShield::load(&recorded, sprite, decode)?;
                    let cells = flag_cells(&recorded)?;
                    layers.note_overlay_refusal(format!("shield {PLAYER_FLAG} ({error})"));
                    (shield, Some((recorded, cells)))
                }
            }
        };
        // A shield without an overlay has no shading to even.
        let panel = match shield.overlay()? {
            Some(overlay) => shield.with_overlay(even_overlay(&overlay)),
            None => shield.clone(),
        };
        Ok(Self {
            shield,
            panel,
            recorded,
            recorded_files: Arc::default(),
            decode,
            warnings,
        })
    }

    /// The top bar's shield as `catalogue`'s presentation binds, declares and loads it. With
    /// visual mods shown it is drawn once too: a shield that loads may still not draw (an effect
    /// the mods break), and the recording's must replace it before any flag is chosen for it.
    fn shown(
        catalogue: &Catalogue,
        decode: installed_content_image::native::Decode,
        warnings: &mut Vec<String>,
    ) -> io::Result<crate::catalogue::MaskedShield> {
        let layers = &catalogue.content.layers;
        let name = installed_content_core::gui::native::window_button_sprite(
            catalogue.guis()?,
            TOP_BAR,
            PLAYER_FLAG,
            warnings,
        )?;
        let sprite = crate::catalogue::created_gui_sprite(&catalogue.sprites, &name)?;
        let shield = crate::catalogue::MaskedShield::load(layers.presentation(), sprite, decode)?;
        if layers.presentation().shows_visual_mods() {
            let cells = flag_cells(layers)?;
            let cell = cells[if shield.small() { 2 } else { 0 }];
            let probe = Rgba8Image {
                width: cell[0],
                height: cell[1],
                pixels: vec![255; (cell[0] * cell[1] * 4) as usize],
            };
            shield.draw(layers.presentation(), &[&probe], decode)?;
        }
        Ok(shield)
    }

    /// The layers and cells the flags this shield frames are read at, with the files kept for
    /// them: the recording's when it is the recording's shield, else the flag context's own.
    fn flags_from<'a>(
        &'a self,
        layers: &'a installed_content_core::ResolvedLayers,
        cells: &'a [[u32; 2]; 3],
        files: &'a Arc<FlagFiles>,
    ) -> (
        &'a installed_content_core::ResolvedLayers,
        &'a [[u32; 2]; 3],
        &'a Arc<FlagFiles>,
    ) {
        match &self.recorded {
            Some((recorded, cells)) => (recorded, cells, &self.recorded_files),
            None => (layers, cells, files),
        }
    }

    /// The flag set the shield samples: the small set when its descriptor says
    /// `small`, else the large set.
    pub(crate) fn sets(&self) -> &'static [FlagSet] {
        if self.shield.small() {
            &[FlagSet::Small]
        } else {
            &[FlagSet::Normal]
        }
    }

    /// Each flag in the shield, `Panel`'s app styling when `panel`, drawn from the stack the
    /// shield was loaded from: the presentation of `layers` or the recording. A recorded
    /// shield names, on every reply it draws, that it stands in for the visual mods'.
    fn draw(
        &self,
        layers: &installed_content_core::ResolvedLayers,
        flags: &[&Rgba8Image],
        panel: bool,
    ) -> io::Result<Vec<Rgba8Image>> {
        let stack = match &self.recorded {
            Some((recorded, _)) => recorded,
            None => layers.presentation(),
        };
        let shield = if panel { &self.panel } else { &self.shield };
        shield.draw(stack, flags, self.decode)
    }
}

/// Whether a recorded flag key names a file this reader can resolve inside a
/// flag folder. The game's path joins the folder, key and `.tga` as text, so
/// a cosmetic tag such as `variants/red` names `gfx/flags/variants/red.tga`.
/// A key of any length is accepted when every `/`-separated part is non-empty,
/// not `.` or `..`, and free of backslashes, NUL and drive colons: so it stays
/// below the folder. Tags are otherwise opaque (spaces, lower case and other
/// bytes are the country's own).
fn safe_flag_key(key: &str) -> bool {
    key.split('/').all(|part| {
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

/// The normal, medium and small flag cells of the graphics defines `layers`' artwork reads
/// (`Defines::graphics_for`): the visual mods' where they set them, else the recording's, named.
pub(crate) fn flag_cells(layers: &installed_content_core::ResolvedLayers) -> io::Result<[[u32; 2]; 3]> {
    let recorded = installed_content_core::defines::read(layers)?;
    let graphics = recorded.graphics_for(layers);
    layers.graphics_define("flag cells", &graphics, &recorded, |defines| {
        Ok::<_, io::Error>([
            FlagSet::Normal.cell(defines)?,
            FlagSet::Medium.cell(defines)?,
            FlagSet::Small.cell(defines)?,
        ])
    })
}

#[derive(Clone)]
enum FlagFile {
    Missing,
    Refused(installed_content_core::flags::DecodeFailure),
    Image(Arc<Rgba8Image>),
}

/// The stack a flag file was read from: the presentation of the layers searched, or the
/// recorded layers below its visual mods.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Stack {
    Shown,
    Recorded,
}

/// What a reply's searches have reported once, kept only with the search that reported it.
#[derive(Clone, Default)]
struct Reported {
    refused: BTreeSet<(Stack, usize, String)>,
    substitutions: BTreeSet<String>,
    unresolvable: BTreeSet<String>,
}

/// One decoding per file/set, shared across the country's key ladders.
#[derive(Default)]
pub(crate) struct FlagCache {
    /// The files read: this search's own, or kept for every search of the same layers.
    files: Arc<FlagFiles>,
    reported: Reported,
}

impl FlagCache {
    /// A search reading through `files`, which other searches of the same layers keep.
    pub(crate) fn over(files: Arc<FlagFiles>) -> Self {
        Self {
            files,
            reported: Reported::default(),
        }
    }

    #[cfg(test)]
    pub(crate) fn files(&self) -> Arc<FlagFiles> {
        Arc::clone(&self.files)
    }
}

/// One stack of layers' flag files as read and decoded, each with the overlay refusals its
/// read noted, which every later use credits again: the files cannot change while mounted.
#[derive(Default)]
pub(crate) struct FlagFiles {
    /// Each file by the stack it was read from, its set, its path and the cell it is decoded at.
    files: Mutex<BTreeMap<(Stack, usize, String, [u32; 2]), (FlagFile, BTreeSet<String>)>>,
    /// The recorded layers below the visual mods, searched when the mods' search fails, and
    /// the cells of the recording's own defines their files are decoded at.
    recorded: OnceLock<(
        (
            installed_content_core::ResolvedLayers,
            Result<[[u32; 2]; 3], String>,
        ),
        BTreeSet<String>,
    )>,
}

/// The file at `asset` of `layers` in set `set_index`, decoded at `cell`, once per `stack`.
fn flag_file(
    files: &FlagFiles,
    stack: Stack,
    layers: &installed_content_core::ResolvedLayers,
    set_index: usize,
    asset: &str,
    cell: [u32; 2],
) -> io::Result<FlagFile> {
    let key = (stack, set_index, asset.to_owned(), cell);
    if let Some((file, noted)) = lock(&files.files).get(&key) {
        installed_content_core::content::credit_overlay_refusals(noted);
        return Ok(file.clone());
    }
    let (file, noted) = installed_content_core::content::observe_overlay_refusals(|| {
        io::Result::Ok(match layers.read_uncovered(asset) {
            Ok(bytes) => match installed_content_core::flags::decode_checked(&bytes, cell) {
                Ok(image) => FlagFile::Image(Arc::new(image)),
                Err(reason) => FlagFile::Refused(reason),
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => FlagFile::Missing,
            Err(error) => return Err(error),
        })
    });
    let file = file?;
    lock(&files.files).insert(key, (file.clone(), noted));
    Ok(file)
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

/// Searches `sets` in order, and in each the candidates in order: a missing
/// file or one the game's decoder refuses falls through to the next candidate.
/// A candidate [`safe_flag_key`] refuses stops the search, because falling
/// past it would draw a flag the game does not.
///
/// Files are read from the presentation of `layers`. With visual mods shown the search is one
/// transaction, as `catalogue::present` draws: the mods' search fails at a candidate whose file
/// they supply but which does not decode, whose read fails, or which they hide or leave
/// undecodable where the recording draws its own. Then the whole ladder is searched again in
/// the recording, at the recording's own cells, so the recording's own choice is made again,
/// and the failing file is named as the map names its own
/// (`ResolvedLayers::note_overlay_refusal`) on every search that falls back. Only the chosen
/// search's findings are returned.
pub(crate) fn find_flag(
    layers: &installed_content_core::ResolvedLayers,
    sets: &[FlagSet],
    cells: &[[u32; 2]; 3],
    candidates: &[String],
    cache: &mut FlagCache,
) -> io::Result<(FlagSearch, Vec<crate::content_check::Finding>)> {
    let FlagCache { files, reported } = cache;
    let files = &**files;
    let presentation = layers.presentation();
    let plain = |files: &FlagFiles, reported: &mut Reported| {
        let Ok(found) = ladder(
            Stack::Shown,
            presentation,
            None,
            sets,
            cells,
            candidates,
            files,
            reported,
        )?
        else {
            unreachable!("only the visual mods' search breaks");
        };
        io::Result::Ok(found)
    };
    if !presentation.shows_visual_mods() {
        return plain(files, reported);
    }
    let ((recorded, recorded_cells), noted) = files.recorded.get_or_init(|| {
        installed_content_core::content::observe_overlay_refusals(|| {
            let recorded = layers.recorded_only();
            let cells = flag_cells(&recorded).map_err(|error| error.to_string());
            (recorded, cells)
        })
    });
    installed_content_core::content::credit_overlay_refusals(noted);
    // Without the recording's own cells nothing can be drawn from the recording: the mods'
    // search stands as it is.
    let Ok(recorded_cells) = recorded_cells.as_ref() else {
        return plain(files, reported);
    };
    let mut shown = reported.clone();
    match ladder(
        Stack::Shown,
        presentation,
        Some((recorded, recorded_cells)),
        sets,
        cells,
        candidates,
        files,
        &mut shown,
    )? {
        Ok(found) => {
            *reported = shown;
            Ok(found)
        }
        Err(Broken { asset, why }) => {
            let Ok(found) = ladder(
                Stack::Recorded,
                recorded,
                None,
                sets,
                recorded_cells,
                candidates,
                files,
                reported,
            )?
            else {
                unreachable!("only the visual mods' search breaks");
            };
            if matches!(found.0, FlagSearch::Found { .. }) {
                layers.note_overlay_refusal(format!("flag {asset} ({why})"));
            }
            Ok(found)
        }
    }
}

/// Where the visual mods' search of a ladder fails: the file they break or hide, and why.
struct Broken {
    asset: String,
    why: String,
}

/// [`find_flag`]'s ladder over `layers`, its files read once per `stack`. Given `recorded` (the
/// recorded layers and their cells) it is the visual mods' search, which fails at the first
/// candidate they break or hide ([`Broken`]); otherwise it never fails that way.
#[allow(clippy::too_many_arguments)]
fn ladder(
    stack: Stack,
    layers: &installed_content_core::ResolvedLayers,
    recorded: Option<(&installed_content_core::ResolvedLayers, &[[u32; 2]; 3])>,
    sets: &[FlagSet],
    cells: &[[u32; 2]; 3],
    candidates: &[String],
    files: &FlagFiles,
    reported: &mut Reported,
) -> io::Result<Result<(FlagSearch, Vec<crate::content_check::Finding>), Broken>> {
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
                if reported.unresolvable.insert(key.clone()) {
                    findings.push(Finding::new(Kind::Refused, key,
                        format!("flag name {key:?} cannot be resolved inside the flag folder, so the country's flag is unavailable")));
                }
                return Ok(Ok((FlagSearch::Unresolvable, findings)));
            }
            let asset = format!("{}{key}.tga", set.directory());
            let read = flag_file(files, stack, layers, set_index, &asset, cell);
            if let Some((recorded, recorded_cells)) = recorded
                && !matches!(read, Ok(FlagFile::Image(_)))
            {
                // A file the mods supply that does not decode, or a read that fails, is theirs
                // to fail, as is one they hide or leave undecodable where the recording draws
                // its own. A file missing or refused alike in both falls through, as the game's
                // search does.
                let breaks = match &read {
                    Ok(FlagFile::Refused(_)) if layers.supplied_by_visual_mod(&asset) => true,
                    Err(_) => true,
                    _ => matches!(
                        flag_file(
                            files,
                            Stack::Recorded,
                            recorded,
                            set_index,
                            &asset,
                            recorded_cells[set_index],
                        ),
                        Ok(FlagFile::Image(_))
                    ),
                };
                if breaks {
                    let why = match &read {
                        Ok(FlagFile::Refused(reason)) => format!("{reason:?}"),
                        Err(error) => error.to_string(),
                        _ => "the visual mods give no such file".to_owned(),
                    };
                    return Ok(Err(Broken { asset, why }));
                }
            }
            match read? {
                FlagFile::Missing => {}
                FlagFile::Refused(reason) => {
                    if !reported.refused.insert((stack, set_index, asset.clone())) {
                        continue;
                    }
                    // The game's decoder rejects these headers and skips the candidate.
                    // This reader also refuses truncated reads.
                    let mut finding = Finding::new(
                        match reason {
                            installed_content_core::flags::DecodeFailure::EngineRefused => Kind::Content,
                            installed_content_core::flags::DecodeFailure::Truncated => Kind::Refused,
                        },
                        key,
                        match reason {
                            installed_content_core::flags::DecodeFailure::EngineRefused => format!(
                                "flag decoder refused {asset} at cell {}x{}; this file was skipped",
                                cell[0], cell[1]
                            ),
                            installed_content_core::flags::DecodeFailure::Truncated => format!(
                                "flag decoder cannot safely read {asset} at cell {}x{}; this reader refuses this input but the game does not check read status",
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
                        && reported.substitutions.insert(asset.clone())
                    {
                        // Medium-to-normal substitution is not established: actionable.
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
                    return Ok(Ok((FlagSearch::Found { asset, image }, findings)));
                }
            }
        }
    }
    // Product omission has no established game display behavior.
    findings.push(Finding::new(
        Kind::Fallback,
        candidates.first().cloned().unwrap_or_default(),
        "no decodable flag in the requested sets; flag artwork is unavailable",
    ));
    Ok(Ok((FlagSearch::Absent, findings)))
}

/// What a drawn flag shows ([`CountryFlagImage::key`]): its file and a digest of its pixels.
pub(crate) fn flag_key(asset: &str, pixels: &[u8]) -> String {
    format!("{asset}#{}", blake3::hash(pixels).to_hex())
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

/// The overlay's coverage averaged over a window a quarter of its shorter side:
/// broad enough to follow the edge shading, narrow enough to leave the weave.
const EVEN_WINDOW_DIVISOR: u32 = 4;

/// `overlay` with its coverage scaled so every area matches the overlay's centre.
/// The vignette that darkens a flag's edges goes; the fine texture keeps its
/// proportions. Colours are unchanged.
fn even_overlay(overlay: &Rgba8Image) -> Rgba8Image {
    let (width, height) = (overlay.width as usize, overlay.height as usize);
    let radius = (overlay.width.min(overlay.height) / EVEN_WINDOW_DIVISOR).max(1) as usize;
    // Summed-area table of alpha, one row and column of zeros in front.
    let mut sums = vec![0_u64; (width + 1) * (height + 1)];
    for y in 0..height {
        for x in 0..width {
            sums[(y + 1) * (width + 1) + x + 1] =
                u64::from(overlay.pixels[(y * width + x) * 4 + 3])
                    + sums[y * (width + 1) + x + 1]
                    + sums[(y + 1) * (width + 1) + x]
                    - sums[y * (width + 1) + x];
        }
    }
    let mean = |x: usize, y: usize| {
        let (left, top) = (x.saturating_sub(radius), y.saturating_sub(radius));
        let (right, bottom) = ((x + radius + 1).min(width), (y + radius + 1).min(height));
        let total = sums[bottom * (width + 1) + right] + sums[top * (width + 1) + left]
            - sums[top * (width + 1) + right]
            - sums[bottom * (width + 1) + left];
        total as f64 / ((right - left) * (bottom - top)) as f64
    };
    let centre = mean(width / 2, height / 2);
    let mut pixels = overlay.pixels.clone();
    for y in 0..height {
        for x in 0..width {
            let local = mean(x, y);
            if local > 0.0 {
                let alpha = &mut pixels[(y * width + x) * 4 + 3];
                *alpha = (f64::from(*alpha) * centre / local)
                    .round()
                    .clamp(0.0, 255.0) as u8;
            }
        }
    }
    Rgba8Image {
        width: overlay.width,
        height: overlay.height,
        pixels,
    }
}

/// The tables [`choices`] reads, its ruling groups included.
const CHOICE_TABLES: [TableId; 5] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::Politics,
    TableId::PoliticalParties,
];

pub struct FlagContext {
    options: ResolveOptions,
    /// The visual mods `content` shows.
    presentation: crate::presentation::Presentation,
    /// The recorded content with the source's visual mods mounted as its presentation, which
    /// flag files are read from.
    content: Arc<ResolvedContent>,
    /// The normal, medium and small cells from the content's defines.
    cells: [[u32; 2]; 3],
    /// What reading `cells` drew from the recording instead of the visual mods' defines,
    /// credited to every reply.
    cell_fallbacks: BTreeSet<String>,
    /// The flag files read from `content`, kept by every context of it.
    files: Arc<FlagFiles>,
    /// The first choice per tag, in raw country id order.
    choices: HashMap<String, FlagChoice>,
    branch_id: String,
    hour: i32,
    warnings: Vec<String>,
}

impl FlagContext {
    fn matches(
        &self,
        options: &ResolveOptions,
        presentation: &crate::presentation::Presentation,
        request: &CountryFlagParams,
    ) -> bool {
        self.presentation == *presentation
            && self.options.game_dir == options.game_dir
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
        let frame =
            match replay.seek_selected_with(branch, request.hour, Some(&CHOICE_TABLES), || {
                crate::metadata::check_cancel(cancelled)
            })? {
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
            presentation: Default::default(),
            content,
            cells,
            cell_fallbacks: BTreeSet::new(),
            files: Arc::default(),
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
        let mut unavailable = Vec::new();
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
                    unavailable: request
                        .tags
                        .iter()
                        .map(|tag| CountryFlagUnavailable {
                            tag: tag.clone(),
                            reason: format!("Shield artwork unavailable: {reason}"),
                        })
                        .collect(),
                    warnings,
                });
            }
            None => None,
        };
        // A recorded shield frames the recording's own flags, at its own cells.
        let (flag_layers, cells, files) = shield_style
            .map_or((&self.content.layers, &self.cells, &self.files), |style| {
                style.flags_from(&self.content.layers, &self.cells, &self.files)
            });
        let mut cache = FlagCache::over(Arc::clone(files));
        if let Some(style) = shield_style {
            warnings.extend(style.warnings.iter().cloned());
        }
        // Found flags, drawn together once every tag is searched.
        let mut shielded = Vec::new();
        for tag in &request.tags {
            if cancelled.load(Ordering::Acquire) {
                return Err(invalid("Flag request cancelled"));
            }
            let Some(choice) = self.choices.get(tag) else {
                warnings.push(format!("{tag}: country identity unavailable at this hour"));
                unavailable.push(CountryFlagUnavailable {
                    tag: tag.clone(),
                    reason: "country identity unavailable at this hour".into(),
                });
                continue;
            };
            // A flag key is the recorded tag (any bytes but NUL, spaces
            // included) with a cosmetic tag or party token; one this reader
            // cannot resolve costs only this country's flag.
            let (found, findings) = find_flag(
                flag_layers,
                shield_style.map_or_else(|| request.size.sets(), ShieldStyle::sets),
                cells,
                &choice.candidates,
                &mut cache,
            )?;
            let reasons: Vec<_> = findings
                .iter()
                .filter(|finding| finding.kind == crate::content_check::Kind::Refused)
                .map(|finding| finding.message.clone())
                .collect();
            warnings.extend(
                findings
                    .into_iter()
                    // Substitution and engine-skipped files stay corpus-only.
                    // Refusals differ from a file the engine could draw.
                    .filter(|finding| finding.kind == crate::content_check::Kind::Refused)
                    .map(|finding| format!("{tag}: {}", finding.message)),
            );
            match found {
                FlagSearch::Found { asset, image } => shielded.push((tag.clone(), asset, image)),
                FlagSearch::Unresolvable => unavailable.push(CountryFlagUnavailable {
                    tag: tag.clone(),
                    reason: if reasons.is_empty() {
                        choice.refusal.clone().unwrap_or_else(|| {
                            "flag artwork unavailable in the recorded content".into()
                        })
                    } else {
                        reasons.join("; ")
                    },
                }),
                FlagSearch::Absent => {
                    warnings.push(match &choice.refusal {
                        Some(reason) => format!("{tag}: flag not drawn: {reason}"),
                        None => format!("{tag}: flag artwork unavailable in the recorded content"),
                    });
                    unavailable.push(CountryFlagUnavailable {
                        tag: tag.clone(),
                        reason: choice.refusal.clone().unwrap_or_else(|| {
                            if reasons.is_empty() {
                                "flag artwork unavailable in the recorded content".into()
                            } else {
                                reasons.join("; ")
                            }
                        }),
                    });
                }
            }
        }
        let drawn = match shield_style {
            Some(style) => {
                let images: Vec<_> = shielded
                    .iter()
                    .map(|(_, _, image)| image.as_ref())
                    .collect();
                style.draw(
                    &self.content.layers,
                    &images,
                    request.size == CountryFlagSize::Panel,
                )
            }
            None => Ok(shielded
                .iter()
                .map(|(_, _, image)| image.as_ref().clone())
                .collect()),
        };
        match drawn {
            Ok(images) => {
                for ((tag, asset, _), image) in shielded.into_iter().zip(images) {
                    let (width, height) = bounded(image.width, image.height, MAX_FLAG_EDGE);
                    let image = resize(&image, width, height);
                    let key = flag_key(&asset, &image.pixels);
                    flags.push(CountryFlagImage {
                        tag,
                        key,
                        asset,
                        width: image.width,
                        height: image.height,
                        pixels: image.pixels,
                    });
                }
            }
            // The shield's passes differ only in the flag they sample, so a
            // refused draw refuses every flag.
            Err(error) => {
                warnings.push(format!("Shield artwork unavailable: {error}"));
                unavailable.extend(shielded.into_iter().map(|(tag, _, _)| {
                    CountryFlagUnavailable {
                        tag,
                        reason: format!("Shield artwork unavailable: {error}"),
                    }
                }));
            }
        }
        Ok(CountryFlagReply {
            branch_id: request.branch_id.clone(),
            hour: request.hour,
            size: request.size,
            flags,
            unavailable,
            warnings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_frame::frame::DecodedFrame;

    fn flag_frame(
        original: replay_data::rows::Recorded<Option<Vec<u8>>>,
        cosmetic: &[u8],
    ) -> DecodedFrame {
        use replay_data::{
            rows::CountryAppearanceRow, rows::CountryLinkRow, rows::CountryRow,
            rows::PoliticalPartyRow, rows::PoliticsRow, rows::TableId, rows::TableRows,
        };
        use replay_frame::{frame::Metrics, frame::TableView};
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 1,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
            }),
        };
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
                id: 10,
                canonical_slot: 2,
            }]));
        *frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![CountryRow {
                id: 10,
                canonical_slot: 2,
                tag: b"OWN".to_vec(),
                ..Default::default()
            }]));
        *frame.tables[TableId::CountryAppearance.index()] =
            TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
                country: 10,
                name_tag: b"GEN".to_vec(),
                cosmetic_tag: cosmetic.to_vec(),
                original_tag: original,
                original_tag_reference: replay_data::rows::Recorded::Value(Some(1)),
                ..Default::default()
            }]));
        *frame.tables[TableId::Politics.index()] =
            TableView::Complete(TableRows::Politics(vec![PoliticsRow {
                country: 10,
                ruling_party: Some(4),
                ..Default::default()
            }]));
        *frame.tables[TableId::PoliticalParties.index()] =
            TableView::Complete(TableRows::PoliticalParties(vec![PoliticalPartyRow {
                country: 10,
                id: 4,
                token: b"group".to_vec(),
                ..Default::default()
            }]));
        frame
    }

    #[test]
    fn flag_choices_use_exact_original_and_keep_known_cosmetic_before_suffix_refusals() {
        use replay_data::rows::Recorded;
        for (captured, tail) in [
            (Recorded::Value(Some(b"ORI".to_vec())), "ORI"),
            (Recorded::Value(Some(Vec::new())), ""),
        ] {
            let choices = choices(&flag_frame(captured, b"COS")).unwrap();
            assert_eq!(
                choices[0].candidates,
                [
                    "COS_group".to_owned(),
                    "COS".to_owned(),
                    format!("{tail}_group"),
                    tail.to_owned()
                ]
            );
            assert_eq!(choices[0].key(|key| key == tail), Ok(tail));
            assert!(choices[0].refusal.is_none());
        }
        for (captured, reason) in [
            (Recorded::Value(None), "original_tag capture refused"),
            (Recorded::NotRecorded, "original_tag not recorded"),
            (Recorded::Value(Some(vec![0xff])), "not UTF-8"),
        ] {
            let choices = choices(&flag_frame(captured, b"COS")).unwrap();
            assert_eq!(choices[0].candidates, ["COS_group", "COS"]);
            assert_eq!(choices[0].key(|key| key == "COS"), Ok("COS"));
            assert!(
                choices[0]
                    .key(|key| key == "OWN" || key == "GEN")
                    .unwrap_err()
                    .contains(reason)
            );
        }
        let invalid =
            choices(&flag_frame(Recorded::Value(Some(b"ORI".to_vec())), &[0xff])).unwrap();
        assert!(invalid[0].candidates.is_empty());
        assert!(
            invalid[0]
                .key(|key| key == "ORI")
                .unwrap_err()
                .contains("cosmetic tag is not UTF-8")
        );
        // A nonpositive raw reference chooses own independently of the
        // unused optional original bytes, including a failed byte capture.
        for reference in [0, -7] {
            let mut frame = flag_frame(Recorded::Value(None), b"COS");
            let appearance =
                match &mut *frame.tables[replay_data::rows::TableId::CountryAppearance.index()] {
                    replay_frame::frame::TableView::Complete(
                        replay_data::rows::TableRows::CountryAppearance(rows),
                    ) => &mut rows[0],
                    _ => unreachable!(),
                };
            appearance.original_tag_reference = Recorded::Value(Some(reference));
            let choice = choices(&frame).unwrap().remove(0);
            assert_eq!(choice.candidates, ["COS_group", "COS", "OWN_group", "OWN"]);
            assert!(choice.refusal.is_none());
        }
        for (reference, reason) in [
            (
                Recorded::Value(None),
                "original_tag_reference capture refused",
            ),
            (Recorded::NotRecorded, "original_tag_reference not recorded"),
        ] {
            let mut frame = flag_frame(Recorded::Value(Some(b"ORI".to_vec())), b"COS");
            let appearance =
                match &mut *frame.tables[replay_data::rows::TableId::CountryAppearance.index()] {
                    replay_frame::frame::TableView::Complete(
                        replay_data::rows::TableRows::CountryAppearance(rows),
                    ) => &mut rows[0],
                    _ => unreachable!(),
                };
            appearance.original_tag_reference = reference;
            let choice = choices(&frame).unwrap().remove(0);
            assert_eq!(choice.candidates, ["COS_group", "COS"]);
            assert_eq!(choice.key(|key| key == "COS"), Ok("COS"));
            assert!(choice.key(|key| key == "ORI").unwrap_err().contains(reason));
        }
    }

    #[test]
    fn flag_choices_preserve_unknown_ruling_groups_before_plain_candidates() {
        use replay_data::{
            rows::PoliticalPartyRow, rows::PoliticsRow, rows::Recorded, rows::TableId,
            rows::TableRows,
        };
        use replay_frame::frame::TableView;
        let mut frame = flag_frame(Recorded::Value(Some(b"ORI".to_vec())), b"COS");
        *frame.tables[TableId::Politics.index()] =
            TableView::Complete(TableRows::Politics(vec![PoliticsRow {
                country: 10,
                ruling_party: Some(4),
                ..Default::default()
            }]));
        for (party, reason) in [
            (None, "needs PoliticalParties ruling group 4"),
            (Some(vec![0xff]), "ruling group is not UTF-8"),
        ] {
            *frame.tables[TableId::PoliticalParties.index()] =
                TableView::Complete(TableRows::PoliticalParties(
                    party
                        .map(|token| PoliticalPartyRow {
                            country: 10,
                            id: 4,
                            token,
                            ..Default::default()
                        })
                        .into_iter()
                        .collect(),
                ));
            let choice = choices(&frame).unwrap().remove(0);
            assert!(choice.candidates.is_empty());
            assert!(
                choice
                    .key(|key| key == "COS" || key == "ORI")
                    .unwrap_err()
                    .contains(reason)
            );
        }
        *frame.tables[TableId::PoliticalParties.index()] =
            TableView::Complete(TableRows::PoliticalParties(vec![PoliticalPartyRow {
                country: 10,
                id: 4,
                token: b"group".to_vec(),
                ..Default::default()
            }]));
        assert_eq!(
            choices(&frame).unwrap()[0].candidates,
            ["COS_group", "COS", "ORI_group", "ORI"]
        );
        // A flag read reconstructs only CHOICE_TABLES: they hold everything choices reads.
        let mut selected = frame.clone();
        for table in TableId::ALL {
            if !CHOICE_TABLES.contains(&table) {
                *selected.tables[table.index()] = TableView::Unavailable {
                    reason: "not selected".into(),
                };
            }
        }
        assert_eq!(
            choices(&selected).unwrap()[0].candidates,
            ["COS_group", "COS", "ORI_group", "ORI"]
        );
        *frame.tables[TableId::PoliticalParties.index()] =
            TableView::Complete(TableRows::PoliticalParties(vec![PoliticalPartyRow {
                country: 10,
                id: 4,
                token: Vec::new(),
                ..Default::default()
            }]));
        assert_eq!(ruling_groups(&frame).unwrap()[&10], Ok(Some(String::new())));
        // The ordinary group's capture identity proves an active flag even with
        // an empty token, so the qualified candidates keep the underscore.
        assert_eq!(
            choices(&frame).unwrap()[0].candidates,
            ["COS_", "COS", "ORI_", "ORI"]
        );
        *frame.tables[TableId::Politics.index()] =
            TableView::Complete(TableRows::Politics(vec![PoliticsRow {
                country: 10,
                ruling_party: None,
                ..Default::default()
            }]));
        assert!(
            choices(&frame).unwrap()[0]
                .key(|key| key == "COS")
                .unwrap_err()
                .contains("null ruling-party pointer")
        );
        *frame.tables[TableId::Politics.index()] = TableView::Unavailable {
            reason: "read failed".into(),
        };
        assert!(
            choices(&frame).unwrap()[0]
                .key(|key| key == "COS")
                .unwrap_err()
                .contains("needs its Politics row")
        );
    }

    #[test]
    fn flag_aliases_use_the_resolved_country_objects_operands() {
        use replay_data::{rows::CountryLinkRow, rows::Recorded, rows::TableId, rows::TableRows};
        use replay_frame::frame::TableView;
        let mut frame = flag_frame(Recorded::Value(Some(b"ORI".to_vec())), b"COS");
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 10,
                    canonical_slot: 2,
                },
                CountryLinkRow {
                    id: 11,
                    canonical_slot: 2,
                },
            ]));
        let choices = choices(&frame).unwrap();
        assert_eq!(
            choices.iter().map(|choice| choice.raw).collect::<Vec<_>>(),
            [10, 11]
        );
        assert_eq!(choices[0].candidates, choices[1].candidates);
        assert!(choices.iter().all(|choice| choice.refusal.is_none()));
        // The resolved object's own link need not occur in a partial alias
        // list for its already captured politics and appearance to be usable.
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
                id: 11,
                canonical_slot: 2,
            }]));
        let alias_only = super::choices(&frame).unwrap();
        assert_eq!(alias_only[0].candidates, choices[0].candidates);
        assert!(alias_only[0].refusal.is_none());
    }

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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
                Some("game does not check read status"),
            ),
            (
                vec!["mod_truncated"],
                None,
                Some("game does not check read status"),
            ),
            (
                vec!["mod_absent"],
                None,
                Some("mod tag: flag artwork unavailable in the recorded content"),
            ),
        ] {
            let context = FlagContext {
                presentation: Default::default(),
                options: ResolveOptions {
                    game_dir: "G".into(),
                    user_dir: "U".into(),
                    cache_dir: "C".into(),
                },
                content: Arc::clone(&catalogue.content),
                cells: [[1, 1]; 3],
                cell_fallbacks: BTreeSet::new(),
                files: Arc::default(),
                choices: HashMap::from([(
                    "mod tag".into(),
                    FlagChoice {
                        raw: 0,
                        tag: "mod tag".into(),
                        candidates: candidates.into_iter().map(str::to_owned).collect(),
                        refusal: None,
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
            if asset.is_some() {
                assert!(reply.unavailable.is_empty());
            } else {
                assert_eq!(reply.unavailable.len(), 1);
                assert_eq!(reply.unavailable[0].tag, "mod tag");
                let reason = reason.unwrap();
                assert!(
                    reply.unavailable[0]
                        .reason
                        .contains(reason.strip_prefix("mod tag: ").unwrap_or(reason))
                );
            }
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
            presentation: Default::default(),
            options: ResolveOptions {
                game_dir: "G".into(),
                user_dir: "U".into(),
                cache_dir: "C".into(),
            },
            content: Arc::clone(&catalogue.content),
            cells: [[1, 1]; 3],
            cell_fallbacks: BTreeSet::new(),
            files: Arc::default(),
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
        assert_eq!(reply.unavailable.len(), 1);
        assert_eq!(reply.unavailable[0].tag, "MOD tag");
        assert_eq!(
            reply.unavailable[0].reason,
            format!("Shield artwork unavailable: {reason}")
        );
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
                    "spriteTypes = { maskedShieldType = { name = GFX_mod_shield textureFile1 = gfx/mod/overlay.png textureFile2 = gfx/mod/mask.png effectFile = gfx/FX/mod.lua } }",
                ),
            ],
            &textures,
        );
        catalogue.gui_decode = Ok(installed_content_image::native::Decode {
            force_pow2_textures: true,
        });
        let loaded = ShieldStyle::load(&catalogue).unwrap();
        assert_eq!(
            [
                loaded.shield.overlay().unwrap().unwrap().width,
                loaded.panel.overlay().unwrap().unwrap().width
            ],
            [4, 4]
        );
        for (path, _, expected) in refusals {
            let facts = installed_content_core::gui::native::sprites::read_sprites(format!(
                "spriteTypes = {{ maskedShieldType = {{ name = GFX_mod_shield textureFile1 = gfx/mod/overlay.png textureFile2 = \"{path}\" effectFile = gfx/FX/mod.lua }} }}"
            ).as_bytes());
            catalogue.sprites =
                installed_content_gfx::gfx::SpriteSet::new(&[("interface/art.gfx".into(), facts)]);
            let why = ShieldStyle::load(&catalogue).err().unwrap().to_string();
            assert!(why.contains(expected), "{path}: {why}");
            assert_shield_warning(&catalogue, &why);
        }
    }

    #[test]
    fn the_shield_s_descriptor_chooses_its_flag_set_and_panel_evens_only_its_overlay() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        use crate::catalogue::texture_tests::{png_bytes, texture_catalogue};
        // An overlay with a dark edge around a lighter centre.
        let overlay = Rgba8Image {
            width: 8,
            height: 8,
            pixels: (0..64)
                .flat_map(|at: u32| {
                    let (x, y) = (at % 8, at / 8);
                    let edge = x.min(y).min(7 - x).min(7 - y) == 0;
                    [10, 10, 10, if edge { 200 } else { 50 }]
                })
                .collect(),
        };
        for (small, sets) in [("no", [FlagSet::Normal]), ("yes", [FlagSet::Small])] {
            let gfx = format!(
                "spriteTypes = {{ maskedShieldType = {{ name = GFX_mod_shield textureFile1 = gfx/mod/overlay.png textureFile2 = gfx/mod/mask.png effectFile = gfx/FX/mod.lua small = {small} }} }}"
            );
            let (catalogue, _folders) = texture_catalogue(
                &[
                    (
                        "interface/topbar.gui",
                        "guiTypes = { containerWindowType = { name = topbar buttonType = { name = player_flag spriteType = GFX_mod_shield } } }",
                    ),
                    ("interface/art.gfx", &gfx),
                ],
                &[
                    ("gfx/mod/mask.png", png_bytes(&overlay)),
                    ("gfx/mod/overlay.png", png_bytes(&overlay)),
                ],
            );
            let style = ShieldStyle::load(&catalogue).unwrap();
            assert_eq!(style.sets(), sets);
            // The native shield keeps its authored overlay; Panel's app styling
            // replaces only the overlay, with its evened copy.
            assert_eq!(style.shield.overlay().unwrap(), Some(overlay.clone()));
            assert_eq!(style.panel.overlay().unwrap(), Some(even_overlay(&overlay)));
        }
    }

    #[test]
    #[ignore = "requires DXC and a GPU; synthetic content only"]
    fn shield_and_panel_flags_draw_the_descriptor_s_flag_set_through_its_passes() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        use crate::catalogue::texture_tests::{png_bytes, shield_programs};
        // A true-colour Targa of one colour, as the flag decoder reads it.
        let flag = |[width, height]: [u32; 2], rgb: [u8; 3]| {
            let mut bytes = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0];
            bytes.extend((width as u16).to_le_bytes());
            bytes.extend((height as u16).to_le_bytes());
            bytes.extend([24, 0x20]);
            for _ in 0..width * height {
                bytes.extend([rgb[2], rgb[1], rgb[0]]);
            }
            bytes
        };
        // An opaque mask under an overlay that is clear but for a dark edge.
        let square = |pixel: &dyn Fn(u32, u32) -> [u8; 4]| Rgba8Image {
            width: 8,
            height: 8,
            pixels: (0..64).flat_map(|at| pixel(at % 8, at / 8)).collect(),
        };
        let mask = square(&|_, _| [255; 4]);
        let overlay = square(&|x, y| {
            let edge = x.min(y).min(7 - x).min(7 - y) == 0;
            [0, 0, 0, if edge { 200 } else { 0 }]
        });
        let programs = shield_programs();
        let mut files: Vec<_> = programs
            .iter()
            .map(|(path, text)| (*path, text.as_str()))
            .collect();
        files.extend([
            (
                "interface/topbar.gui",
                "guiTypes = { containerWindowType = { name = topbar buttonType = { name = player_flag spriteType = GFX_mod_shield } } }",
            ),
            (
                "interface/art.gfx",
                "spriteTypes = { maskedShieldType = { name = GFX_mod_shield textureFile1 = gfx/mod/overlay.png textureFile2 = gfx/mod/mask.png effectFile = gfx/FX/shield_flag.lua small = yes } }",
            ),
        ]);
        let (_, folder) = crate::catalogue::tests::folder_catalogue(&files);
        let cells = [[4, 2], [2, 2], [2, 1]];
        for (path, bytes) in [
            ("gfx/mod/mask.png", png_bytes(&mask)),
            ("gfx/mod/overlay.png", png_bytes(&overlay)),
            ("gfx/flags/MOD.tga", flag(cells[0], [0, 0, 200])),
            ("gfx/flags/small/MOD.tga", flag(cells[2], [0, 200, 0])),
        ] {
            let file = folder.0.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, bytes).unwrap();
        }
        // Flags are covered content: the stack is resolved once they are written.
        let catalogue = crate::catalogue::tests::layers_catalogue(
            installed_content_core::ResolvedLayers::from_folders(&[(&folder.0, &[][..])]).unwrap(),
        );
        let style = ShieldStyle::load(&catalogue).unwrap();
        let context = FlagContext {
            presentation: Default::default(),
            options: ResolveOptions {
                game_dir: "G".into(),
                user_dir: "U".into(),
                cache_dir: "C".into(),
            },
            content: Arc::clone(&catalogue.content),
            cells,
            cell_fallbacks: BTreeSet::new(),
            files: Arc::default(),
            choices: HashMap::from([(
                "MOD".to_owned(),
                FlagChoice {
                    raw: 0,
                    tag: "MOD".into(),
                    candidates: vec!["MOD".into()],
                    refusal: None,
                },
            )]),
            branch_id: "1".into(),
            hour: 0,
            warnings: Vec::new(),
        };
        let pixel = |reply: &CountryFlagReply, x: u32, y: u32| {
            let image = &reply.flags[0];
            let at = ((y * image.width + x) * 4) as usize;
            <[u8; 4]>::try_from(&image.pixels[at..at + 4]).unwrap()
        };
        let mut drawn = Vec::new();
        for size in [CountryFlagSize::Shield, CountryFlagSize::Panel] {
            let request = CountryFlagParams {
                branch_id: "1".into(),
                hour: 0,
                tags: vec!["MOD".into()],
                size,
            };
            let reply = context
                .reply(&request, Some(Ok(&style)), &AtomicBool::new(false))
                .unwrap();
            assert!(reply.unavailable.is_empty(), "{:?}", reply.unavailable);
            // `small = yes`: the small flag set's green, never the large blue.
            assert_eq!(reply.flags[0].asset, "gfx/flags/small/MOD.tga");
            assert_eq!([reply.flags[0].width, reply.flags[0].height], [8, 8]);
            assert_eq!(pixel(&reply, 4, 4), [0, 200, 0, 255]);
            drawn.push(pixel(&reply, 0, 4));
        }
        // The native edge darkens under the overlay; Panel's app styling evens it away.
        let near = |a: [u8; 4], b: [u8; 4]| a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 1);
        assert!(near(drawn[0], [0, 43, 0, 255]), "{:?}", drawn[0]);
        assert!(near(drawn[1], [0, 200, 0, 255]), "{:?}", drawn[1]);
    }

    /// #660's shield fixture as a recorded folder: a small-set shield whose opaque mask sits
    /// under an overlay clear but for a dark edge, and the green small flag it frames.
    fn recorded_shield() -> (crate::catalogue::tests::Folder, [[u32; 2]; 3]) {
        use crate::catalogue::texture_tests::shield_programs;
        let programs = shield_programs();
        let mut files: Vec<_> = programs
            .iter()
            .map(|(path, text)| (*path, text.as_str()))
            .collect();
        files.extend([
            // The cells the flags below are drawn at, as the recording's defines give them.
            (
                "common/defines/00_flags.lua",
                "NDefines = { NGraphics = { COUNTRY_FLAG_TEX_WIDTH = 4, COUNTRY_FLAG_TEX_HEIGHT = 2, COUNTRY_FLAG_TEX_MAX_SIZE = 8, COUNTRY_FLAG_MEDIUM_TEX_WIDTH = 2, COUNTRY_FLAG_MEDIUM_TEX_HEIGHT = 2, COUNTRY_FLAG_MEDIUM_TEX_MAX_SIZE = 8, COUNTRY_FLAG_SMALL_TEX_WIDTH = 2, COUNTRY_FLAG_SMALL_TEX_HEIGHT = 1, COUNTRY_FLAG_SMALL_TEX_MAX_SIZE = 8 } }",
            ),
            (
                "interface/topbar.gui",
                "guiTypes = { containerWindowType = { name = topbar buttonType = { name = player_flag spriteType = GFX_mod_shield } } }",
            ),
            (
                "interface/art.gfx",
                "spriteTypes = { maskedShieldType = { name = GFX_mod_shield textureFile1 = gfx/mod/overlay.png textureFile2 = gfx/mod/mask.png effectFile = gfx/FX/shield_flag.lua small = yes } }",
            ),
        ]);
        let (_, folder) = crate::catalogue::tests::folder_catalogue(&files);
        let cells = [[4, 2], [2, 2], [2, 1]];
        let mut written = vec![
            ("gfx/mod/mask.png", shield_square(&|_, _| [255; 4])),
            (
                "gfx/mod/overlay.png",
                shield_square(&|x, y| {
                    let edge = x.min(y).min(7 - x).min(7 - y) == 0;
                    [0, 0, 0, if edge { 200 } else { 0 }]
                }),
            ),
        ];
        written.extend([
            ("gfx/flags/MOD.tga", shield_flag(cells[0], [0, 0, 200])),
            (
                "gfx/flags/small/MOD.tga",
                shield_flag(cells[2], [0, 200, 0]),
            ),
        ]);
        for (path, bytes) in written {
            let file = folder.0.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, bytes).unwrap();
        }
        (folder, cells)
    }

    /// An 8 by 8 PNG of `pixel`.
    fn shield_square(pixel: &dyn Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        crate::catalogue::texture_tests::png_bytes(&Rgba8Image {
            width: 8,
            height: 8,
            pixels: (0..64).flat_map(|at| pixel(at % 8, at / 8)).collect(),
        })
    }

    /// A true-colour Targa of one colour, as the flag decoder reads it.
    fn shield_flag([width, height]: [u32; 2], rgb: [u8; 3]) -> Vec<u8> {
        let mut bytes = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        bytes.extend((width as u16).to_le_bytes());
        bytes.extend((height as u16).to_le_bytes());
        bytes.extend([24, 0x20]);
        for _ in 0..width * height {
            bytes.extend([rgb[2], rgb[1], rgb[0]]);
        }
        bytes
    }

    /// The shield `MOD`'s flag is drawn in from `layers` (the recorded folder, with any visual
    /// mod mounted over it), as the centre and edge pixels of its Shield reply, with what the
    /// load and the reply named as drawn from the recording.
    fn shield_pixels(
        layers: installed_content_core::ResolvedLayers,
        cells: [[u32; 2]; 3],
    ) -> ([u8; 4], [u8; 4], BTreeSet<String>) {
        let ((centre, edge), fallbacks) = installed_content_core::content::observe_overlay_refusals(|| {
            let catalogue = Catalogue::of_layers(layers, "english").unwrap();
            let style = ShieldStyle::load(&catalogue).unwrap();
            let context = FlagContext {
                presentation: Default::default(),
                options: ResolveOptions {
                    game_dir: "G".into(),
                    user_dir: "U".into(),
                    cache_dir: "C".into(),
                },
                content: Arc::clone(&catalogue.content),
                cells,
                cell_fallbacks: BTreeSet::new(),
                files: Arc::default(),
                choices: HashMap::from([(
                    "MOD".to_owned(),
                    FlagChoice {
                        raw: 0,
                        tag: "MOD".into(),
                        candidates: vec!["MOD".into()],
                        refusal: None,
                    },
                )]),
                branch_id: "1".into(),
                hour: 0,
                warnings: Vec::new(),
            };
            let request = CountryFlagParams {
                branch_id: "1".into(),
                hour: 0,
                tags: vec!["MOD".into()],
                size: CountryFlagSize::Shield,
            };
            let reply = context
                .reply(&request, Some(Ok(&style)), &AtomicBool::new(false))
                .unwrap();
            assert!(reply.unavailable.is_empty(), "{:?}", reply.unavailable);
            let image = &reply.flags[0];
            let pixel = |x: u32, y: u32| {
                let at = ((y * image.width + x) * 4) as usize;
                <[u8; 4]>::try_from(&image.pixels[at..at + 4]).unwrap()
            };
            (pixel(4, 4), pixel(0, 4))
        });
        (centre, edge, fallbacks)
    }

    /// `layers` with the folder mod at `mod_root` selected as a visual mod, installed through a
    /// user folder under `user`.
    fn with_visual_mod(
        mut layers: installed_content_core::ResolvedLayers,
        user: &std::path::Path,
        mod_root: &std::path::Path,
    ) -> installed_content_core::ResolvedLayers {
        let descriptor = user.join("mod/shield.mod");
        std::fs::create_dir_all(descriptor.parent().unwrap()).unwrap();
        std::fs::write(
            &descriptor,
            format!(
                "name=\"Shield\"\npath=\"{}\"\n",
                mod_root.display().to_string().replace('\\', "/")
            ),
        )
        .unwrap();
        let options = ResolveOptions {
            game_dir: user.to_owned(),
            user_dir: user.to_owned(),
            cache_dir: user.join("cache"),
        };
        let choice = installed_content_core::content::visual::Choice {
            id: "mod/shield.mod".into(),
            name: "Shield".into(),
        };
        let plan = installed_content_core::content::visual::apply(
            &mut layers,
            &options,
            &[choice],
            &[],
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);
        layers
    }

    /// The topbar shield follows the selected visual mods: their own shield draws, one whose
    /// textures they do not supply is the recording's own declaration, and one that loads but
    /// does not draw is the recording's drawn shield, each named; mods that leave the shield
    /// alone, and no mods at all, draw the recording's shield exactly as before, naming nothing.
    #[test]
    fn the_shield_follows_the_selected_visual_mods_and_falls_back_by_name() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let near = |a: [u8; 4], b: [u8; 4]| a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 1);
        let (recorded, cells) = recorded_shield();
        let base =
            || installed_content_core::ResolvedLayers::from_folders(&[(&recorded.0, &[][..])]).unwrap();
        // No visual mods: #660's draw, the dark edge shaded in, nothing named.
        let (centre, edge, fallbacks) = shield_pixels(base(), cells);
        assert_eq!(centre, [0, 200, 0, 255]);
        assert!(near(edge, [0, 43, 0, 255]), "{edge:?}");
        assert!(fallbacks.is_empty(), "{fallbacks:?}");
        let visual = |name: &str, files: &[(&str, Vec<u8>)]| {
            let (_, root) = crate::catalogue::tests::folder_catalogue(&[]);
            for (path, bytes) in files {
                let file = root.0.join(name).join(path);
                std::fs::create_dir_all(file.parent().unwrap()).unwrap();
                std::fs::write(file, bytes).unwrap();
            }
            let layers = with_visual_mod(base(), &root.0.join("user"), &root.0.join(name));
            shield_pixels(layers, cells)
        };
        let named = |name: &str, overlay: &str, mask: &str, effect: &str, small: &str| {
            format!(
                "spriteTypes = {{ maskedShieldType = {{ name = {name} textureFile1 = {overlay} textureFile2 = {mask} effectFile = {effect} small = {small} }} }}"
            )
            .into_bytes()
        };
        let declared = |overlay: &str, mask: &str, effect: &str| {
            named("GFX_mod_shield", overlay, mask, effect, "yes")
        };
        let fell_back = |fallbacks: &BTreeSet<String>| {
            fallbacks
                .iter()
                .any(|why| why.starts_with("shield player_flag ("))
        };
        // The mod's own shield, an overlay without the dark edge: the edge stays green.
        let clear = shield_square(&|_, _| [0, 0, 0, 0]);
        let (centre, edge, fallbacks) = visual(
            "own",
            &[
                (
                    "interface/art.gfx",
                    declared(
                        "gfx/visual/overlay.png",
                        "gfx/mod/mask.png",
                        "gfx/FX/shield_flag.lua",
                    ),
                ),
                ("gfx/visual/overlay.png", clear),
            ],
        );
        assert_eq!((centre, edge), ([0, 200, 0, 255], [0, 200, 0, 255]));
        assert!(fallbacks.is_empty(), "{fallbacks:?}");
        // A declaration whose mask the mod does not supply: the recording's own shield.
        let (centre, edge, fallbacks) = visual(
            "missing",
            &[(
                "interface/art.gfx",
                declared(
                    "gfx/mod/overlay.png",
                    "gfx/visual/absent_mask.png",
                    "gfx/FX/shield_flag.lua",
                ),
            )],
        );
        assert_eq!(centre, [0, 200, 0, 255]);
        assert!(near(edge, [0, 43, 0, 255]), "{edge:?}");
        assert!(fell_back(&fallbacks), "{fallbacks:?}");
        // A mod that binds the top bar to a shield of its own name, which does not load: the
        // recording's own binding and declaration, not the mod's name looked up in the
        // recording.
        let (centre, edge, fallbacks) = visual(
            "renamed",
            &[
                (
                    "interface/topbar.gui",
                    b"guiTypes = { containerWindowType = { name = topbar buttonType = { name = player_flag spriteType = GFX_visual_shield } } }"
                        .to_vec(),
                ),
                (
                    "interface/visual.gfx",
                    named(
                        "GFX_visual_shield",
                        "gfx/mod/overlay.png",
                        "gfx/visual/absent_mask.png",
                        "gfx/FX/shield_flag.lua",
                        "yes",
                    ),
                ),
            ],
        );
        assert_eq!(centre, [0, 200, 0, 255]);
        assert!(near(edge, [0, 43, 0, 255]), "{edge:?}");
        assert!(fell_back(&fallbacks), "{fallbacks:?}");
        // A mod that restyles other artwork and leaves the shield alone: the stack resolves the
        // recording's own shield, a plain draw with nothing named.
        let (centre, edge, fallbacks) = visual(
            "untouched",
            &[
                (
                    "interface/other.gfx",
                    b"spriteTypes = { spriteType = { name = GFX_other textureFile = gfx/visual/other.png } }"
                        .to_vec(),
                ),
                ("gfx/visual/other.png", shield_square(&|_, _| [9, 9, 9, 255])),
            ],
        );
        assert_eq!(centre, [0, 200, 0, 255]);
        assert!(near(edge, [0, 43, 0, 255]), "{edge:?}");
        assert!(fallbacks.is_empty(), "{fallbacks:?}");
        // A shield that loads but whose effect does not draw, from the large flag set: the
        // recording's drawn shield, framing the recording's own small-set flag (green, not the
        // large set's blue) at its own cells.
        let (centre, edge, fallbacks) = visual(
            "undrawn",
            &[(
                "interface/art.gfx",
                named(
                    "GFX_mod_shield",
                    "gfx/mod/overlay.png",
                    "gfx/mod/mask.png",
                    "gfx/FX/absent_flag.lua",
                    "no",
                ),
            )],
        );
        assert_eq!(centre, [0, 200, 0, 255]);
        assert!(near(edge, [0, 43, 0, 255]), "{edge:?}");
        assert!(fell_back(&fallbacks), "{fallbacks:?}");
    }

    /// A kept shield is served only for the catalogue it came from: once the visual mods change,
    /// the new presentation's catalogue loads its own, never the old one's.
    #[test]
    fn a_kept_shield_belongs_to_the_catalogue_it_came_from() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (recorded, _) = recorded_shield();
        let base =
            || installed_content_core::ResolvedLayers::from_folders(&[(&recorded.0, &[][..])]).unwrap();
        let (_, root) = crate::catalogue::tests::folder_catalogue(&[]);
        let broken = root.0.join("broken");
        std::fs::create_dir_all(broken.join("interface")).unwrap();
        std::fs::write(
            broken.join("interface/art.gfx"),
            "spriteTypes = { maskedShieldType = { name = GFX_mod_shield textureFile1 = gfx/mod/overlay.png textureFile2 = gfx/mod/mask.png effectFile = gfx/FX/absent_flag.lua small = yes } }",
        )
        .unwrap();
        let shown = Catalogue::of_layers(base(), "english").unwrap();
        let selected = Catalogue::of_layers(
            with_visual_mod(base(), &root.0.join("user"), &broken),
            "english",
        )
        .unwrap();
        let mut slot = None;
        let first = cached_shield(&mut slot, &shown).unwrap();
        assert!(first.recorded.is_none());
        assert!(Arc::ptr_eq(
            &first,
            &cached_shield(&mut slot, &shown).unwrap()
        ));
        let switched = cached_shield(&mut slot, &selected).unwrap();
        assert!(
            !Arc::ptr_eq(&first, &switched),
            "another catalogue loads its own"
        );
        assert!(
            switched.recorded.is_some(),
            "the selected mods' broken shield falls back"
        );
    }

    #[test]
    fn sizes_search_the_game_flag_sets_in_order() {
        assert_eq!(
            CountryFlagSize::Medium.sets(),
            [FlagSet::Medium, FlagSet::Normal]
        );
        assert!(CountryFlagSize::Shield.sets().is_empty());
        assert!(CountryFlagSize::Panel.sets().is_empty());
        assert_eq!(bounded(82, 52, 128), (82, 52));
    }

    #[test]
    fn panel_overlay_loses_its_edge_shading_and_keeps_its_texture() {
        // A 16x16 overlay: alpha 150 within two pixels of the edge, otherwise a
        // weave alternating 40 and 60 around a centre mean of 50.
        let mut pixels = Vec::new();
        for y in 0..16_u32 {
            for x in 0..16_u32 {
                let edge = x.min(y).min(15 - x).min(15 - y) < 2;
                let alpha = if edge {
                    150
                } else if (x + y) % 2 == 0 {
                    40
                } else {
                    60
                };
                pixels.extend_from_slice(&[10, 10, 10, alpha]);
            }
        }
        let overlay = Rgba8Image {
            width: 16,
            height: 16,
            pixels,
        };
        let even = even_overlay(&overlay);
        let alpha = |image: &Rgba8Image, x: u32, y: u32| {
            u32::from(image.pixels[((y * 16 + x) * 4 + 3) as usize])
        };
        for (x, y) in [(0, 0), (0, 8), (15, 15)] {
            assert!(
                alpha(&even, x, y) * 5 < alpha(&overlay, x, y) * 3,
                "the dark edge at ({x}, {y}) is evened out"
            );
        }
        assert_eq!(
            (alpha(&even, 7, 8), alpha(&even, 8, 8)),
            (alpha(&overlay, 7, 8), alpha(&overlay, 8, 8)),
            "the centre's texture is unchanged"
        );
        assert!(
            alpha(&even, 5, 4) > alpha(&even, 4, 4),
            "the weave keeps its contrast"
        );
        assert_eq!(
            &even.pixels[..3],
            &overlay.pixels[..3],
            "colours are unchanged"
        );
    }

    #[test]
    fn flag_params_default_to_full_and_reject_invalid_tags_and_unbounded_batches() {
        let value = serde_json::json!({ "branchId": "1", "hour": 0, "tags": ["GER"] });
        let params: CountryFlagParams = serde_json::from_value(value).unwrap();
        assert_eq!(params.size, CountryFlagSize::Full);
        assert!(CountryFlags::validate(&params).is_ok());
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
