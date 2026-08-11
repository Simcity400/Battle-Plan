//! Machine-local playback profile selection.
//!
//! A `.battleplan` never contains these paths.  It carries a portable content
//! manifest; these two roots merely tell this installation of the standalone
//! replayer where to reconstruct the launcher's effective VFS for validation
//! and playback.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::Result;

const CONFIG_SCHEMA_VERSION: u32 = 1;
const CONFIG_DIR: &str = "battle-plan";
const CONFIG_FILE: &str = "playback_profile.json";
const LEGACY_CONFIG_FILE: &str = "product_paths.json";
#[cfg(any(windows, test))]
const RECORDER_DATA_DIRECTORY_NAME: &str = "Battle Plan Recorder";
const PRODUCT_PROFILE_SETTING_KEYS: [&str; 11] = [
    "physical_width",
    "physical_height",
    "min_gui_width",
    "min_gui_height",
    "gui_scale_f32_bits",
    "refresh_rate_hz",
    "max_refresh_rate_hz",
    "fullscreen",
    "borderless",
    "display_mode",
    "settings_file_sha256",
];
static PROFILE_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// The only machine-local values retained by the replayer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductPaths {
    pub game_install: PathBuf,
    pub user_data: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct StoredProfile {
    schema_version: u32,
    game_install: PathBuf,
    user_data: PathBuf,
}

/// How the profile resolver should source a profile for this invocation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResolveOptions {
    pub game_override: Option<PathBuf>,
    pub user_data_override: Option<PathBuf>,
    /// Ignore a saved profile and ask for any root without an explicit override.
    pub force_reselect: bool,
    /// Never read stdin. Missing or invalid data produces an actionable error.
    pub non_interactive: bool,
}

impl From<ProductPaths> for StoredProfile {
    fn from(value: ProductPaths) -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            game_install: value.game_install,
            user_data: value.user_data,
        }
    }
}

impl From<StoredProfile> for ProductPaths {
    fn from(value: StoredProfile) -> Self {
        Self {
            game_install: value.game_install,
            user_data: value.user_data,
        }
    }
}

/// Select the default configuration file without assuming a game install or
/// user-data location. `battle-plan_CONFIG` is the portable/headless override.
pub fn default_config_path() -> Result<PathBuf> {
    if let Some(path) = env::var_os("battle-plan_CONFIG") {
        if path.is_empty() {
            return Err("battle-plan_CONFIG is set but empty".into());
        }
        return Ok(PathBuf::from(path));
    }
    #[cfg(windows)]
    return Ok(config_path_from_local_app_data(&local_app_data_root()?));

    #[cfg(not(windows))]
    {
        if let Some(root) = env::var_os("XDG_CONFIG_HOME") {
            return Ok(PathBuf::from(root).join(CONFIG_DIR).join(CONFIG_FILE));
        }
        if let Some(root) = env::var_os("HOME") {
            return Ok(PathBuf::from(root)
                .join(".config")
                .join(CONFIG_DIR)
                .join(CONFIG_FILE));
        }
        Err(
            "cannot choose a configuration location; pass --config <file> or set battle-plan_CONFIG"
                .into(),
        )
    }
}

#[cfg(any(windows, test))]
fn config_path_from_local_app_data(root: &Path) -> PathBuf {
    root.join(CONFIG_DIR).join(CONFIG_FILE)
}

#[cfg(any(windows, test))]
fn recorder_recordings_root_from_local_app_data(root: &Path) -> PathBuf {
    root.join(RECORDER_DATA_DIRECTORY_NAME)
}

/// Return the recorder's ordinary per-user output root without creating it.
/// The no-argument viewer uses this only as its file-picker starting folder.
#[cfg(windows)]
pub fn recorder_recordings_root() -> Result<PathBuf> {
    Ok(recorder_recordings_root_from_local_app_data(
        &local_app_data_root()?,
    ))
}

/// Resolve machine-local storage through the Windows Known Folder API. Neither
/// APPDATA nor LOCALAPPDATA environment variables participate in this path.
#[cfg(windows)]
fn local_app_data_root() -> Result<PathBuf> {
    use std::ffi::{c_void, OsString};
    use std::os::windows::ffi::OsStringExt;
    use std::ptr;

    #[repr(C)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    const FOLDERID_LOCAL_APP_DATA: Guid = Guid {
        data1: 0xF1B3_2785,
        data2: 0x6FBA,
        data3: 0x4FCF,
        data4: [0x9D, 0x55, 0x7B, 0x8E, 0x7F, 0x15, 0x70, 0x91],
    };

    #[link(name = "shell32")]
    extern "system" {
        fn SHGetKnownFolderPath(
            folder_id: *const Guid,
            flags: u32,
            token: *mut c_void,
            path: *mut *mut u16,
        ) -> i32;
    }

    #[link(name = "ole32")]
    extern "system" {
        fn CoTaskMemFree(memory: *mut c_void);
    }

    unsafe {
        let mut raw: *mut u16 = ptr::null_mut();
        let result = SHGetKnownFolderPath(&FOLDERID_LOCAL_APP_DATA, 0, ptr::null_mut(), &mut raw);
        if result < 0 || raw.is_null() {
            return Err(format!(
                "resolve LocalAppData Known Folder failed: HRESULT 0x{:08x}; pass --config <file> or set battle-plan_CONFIG",
                result as u32
            )
            .into());
        }
        let mut len = 0usize;
        while *raw.add(len) != 0 {
            if len >= 32_767 {
                CoTaskMemFree(raw.cast());
                return Err("LocalAppData Known Folder path exceeds Windows limit".into());
            }
            len += 1;
        }
        let root = PathBuf::from(OsString::from_wide(std::slice::from_raw_parts(raw, len)));
        CoTaskMemFree(raw.cast());
        if root.as_os_str().is_empty() || !root.is_absolute() {
            return Err("LocalAppData Known Folder returned a non-absolute path".into());
        }
        Ok(root)
    }
}

pub fn legacy_config_path(config: &Path) -> PathBuf {
    config.with_file_name(LEGACY_CONFIG_FILE)
}

/// Load, validate, interactively repair if allowed, and atomically persist a
/// playback profile. Explicit roots always win over saved roots.
pub fn resolve_or_prompt<R: BufRead, W: Write>(
    config: &Path,
    options: ResolveOptions,
    input: &mut R,
    output: &mut W,
) -> Result<ProductPaths> {
    let saved = if options.force_reselect {
        None
    } else {
        load_with_migration(config)?
    };

    let mut game_seed = options
        .game_override
        .or_else(|| saved.as_ref().map(|value| value.game_install.clone()));
    let mut user_seed = options
        .user_data_override
        .or_else(|| saved.as_ref().map(|value| value.user_data.clone()));

    loop {
        let game_install = resolve_root(
            "the game install/content directory",
            game_seed.take(),
            options.non_interactive,
            input,
            output,
            validate_game_install,
        )?;
        let user_data = resolve_root(
            "the game user-data/profile directory",
            user_seed.take(),
            options.non_interactive,
            input,
            output,
            validate_user_data,
        )?;
        let paths = ProductPaths {
            game_install,
            user_data,
        };
        match validate_product_profile(&paths) {
            Ok(_) => {
                store_atomic(config, &paths)?;
                return Ok(paths);
            }
            Err(error) if options.non_interactive => {
                return Err(format!(
                    "the selected roots do not produce a valid launcher-equivalent content profile: {error}"
                )
                .into())
            }
            Err(error) => {
                writeln!(
                    output,
                    "Saved/selected playback profile is invalid: {error}; select both roots again"
                )?;
            }
        }
    }
}

pub fn resolve_process_paths(
    config_override: Option<PathBuf>,
    options: ResolveOptions,
) -> Result<ProductPaths> {
    let config = match config_override {
        Some(path) => path,
        None => default_config_path()?,
    };
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    resolve_or_prompt(&config, options, &mut input, &mut output)
}

pub fn validate_game_install(path: &Path) -> std::result::Result<(), String> {
    if !path.is_dir() {
        return Err(format!("directory does not exist: {}", path.display()));
    }
    // Validate the content root, not a platform-specific executable or a
    // Steam/default-drive location. These are launcher-independent the game roots.
    let required = ["common", "gfx", "map"];
    let missing = required
        .iter()
        .filter(|name| !path.join(name).is_dir())
        .copied()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(format!(
            "not a the game content root; missing directories: {}",
            missing.join(", ")
        ));
    }
    Ok(())
}

pub fn validate_user_data(path: &Path) -> std::result::Result<(), String> {
    if !path.is_dir() {
        return Err(format!("directory does not exist: {}", path.display()));
    }
    let selection = path.join("dlc_load.json");
    if !selection.is_file() {
        return Err(format!(
            "profile has no dlc_load.json launcher selection: {}",
            selection.display()
        ));
    }
    Ok(())
}

/// Validate every machine-local input needed to reconstruct the launcher's
/// effective content and material-settings profile. Callers must complete this
/// check before persisting or reusing a product profile.
pub fn validate_product_profile(paths: &ProductPaths) -> Result<map_viewer::GameFileResolver> {
    let resolver = map_viewer::GameFileResolver::from_dlc_load(
        paths.game_install.clone(),
        paths.user_data.clone(),
    )
    .map_err(|error| format!("resolve launcher content selection: {error}"))?;
    playback_environment_from_paths(
        paths,
        &resolver,
        PRODUCT_PROFILE_SETTING_KEYS.into_iter().map(str::to_string),
    )
    .map_err(|error| format!("validate selected playback profile metadata: {error}"))?;
    Ok(resolver)
}

/// Derive exact, path-redacted local facts for portable-content admission.
/// Expected values from the replay are never used as local defaults.
pub fn playback_environment_from_paths(
    paths: &ProductPaths,
    resolver: &map_viewer::GameFileResolver,
    required_setting_keys: impl IntoIterator<Item = String>,
) -> Result<map_viewer::PlaybackEnvironment> {
    let source_game = source_game_identity(&paths.game_install)?;
    let settings_path = paths.user_data.join("settings.txt");
    let settings_bytes = fs::read(&settings_path)
        .map_err(|error| format!("read selected profile {}: {error}", settings_path.display()))?;
    let settings_text = std::str::from_utf8(&settings_bytes)
        .map_err(|error| format!("selected profile settings.txt is not UTF-8: {error}"))?;
    let graphics = named_block(settings_text, "graphics")?;
    let size = named_block(graphics, "size")?;
    let min_gui = named_block(graphics, "min_gui")?;

    let settings_language = assignment(settings_text, "language")?
        .trim_matches('"')
        .to_string();
    if settings_language.trim().is_empty() {
        return Err("settings.txt language is blank".into());
    }
    let pdx_path = paths.user_data.join("pdx_settings.txt");
    let pdx = match fs::read_to_string(&pdx_path) {
        Ok(pdx) => Some(pdx),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(format!(
                "optional selected-profile file {} is present but unreadable: {error}",
                pdx_path.display()
            )
            .into())
        }
    };
    let localisation_language = if let Some(pdx) = pdx {
        let system = named_block(&pdx, "System")?;
        let language = named_block(system, "language")?;
        let selected = assignment(language, "value")?.trim_matches('"').to_string();
        if selected != settings_language {
            return Err(format!(
                "selected profile language is inconsistent: pdx_settings.txt has {selected:?}, settings.txt has {settings_language:?}"
            )
            .into());
        }
        selected
    } else {
        settings_language
    };

    let full_settings = exact_material_settings(graphics, size, min_gui, &settings_bytes)?;
    let mut settings = BTreeMap::new();
    for key in required_setting_keys {
        let value = full_settings.get(&key).ok_or_else(|| {
            format!(
                "replay requires material setting {key:?}, but this replayer has no proven parser for it"
            )
        })?;
        settings.insert(key, value.clone());
    }

    let dlc_selection = resolver
        .active_sources()
        .into_iter()
        .filter(|source| source.kind == "dlc")
        .map(|source| {
            Ok(replay_core::DlcSelection {
                dlc_id: replay_core::normalize_dlc_selection_id(&source.source_id)
                    .map_err(|error| format!("selected DLC layer identity is invalid: {error}"))?,
                enabled: true,
                required_for_playback: false,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let selection_seed = serde_json::json!({
        "layers": resolver.active_sources(),
        "dlc_selection": dlc_selection,
        "localisation_language": localisation_language,
        "settings": settings,
    });
    let selection_sha256 = format!("{:x}", Sha256::digest(serde_json::to_vec(&selection_seed)?));
    let source_profile = replay_core::SourceProfileIdentity {
        profile_id: format!("runtime-profile:{selection_sha256}"),
        playset_id: format!("runtime-playset:{selection_sha256}"),
        selection_sha256,
    };

    Ok(map_viewer::PlaybackEnvironment {
        source_game,
        source_profile,
        dlc_selection,
        localisation_language,
        settings,
    })
}

/// Admit the standalone viewer's own UI presentation profile.
///
/// Unlike portable-content compatibility, this always requests every local
/// setting that affects UI layout. No value is read from replay session
/// metadata, and the input family is the one implemented by the Win32 client
/// event pump (mouse + keyboard, with no invented traversal/controller graph).
pub fn playback_ui_profile_from_paths(
    paths: &ProductPaths,
    resolver: &map_viewer::GameFileResolver,
    actual_client_input: game_ui::variant_admission::InputVariant,
) -> Result<game_ui::variant_admission::PlaybackUiProfile> {
    let environment = playback_environment_from_paths(
        paths,
        resolver,
        PRODUCT_PROFILE_SETTING_KEYS.into_iter().map(str::to_string),
    )?;
    game_ui::variant_admission::PlaybackUiProfile::admit(&environment, actual_client_input)
        .map_err(|error| format!("admit local playback UI profile: {error:?}").into())
}

/// `draw_hires_terrain` byte from the `mapRenderingOptions` settings object;
/// replay must use the selected profile value rather than a recorder carrier
/// or a hard-coded default.
pub fn playback_terrain_lod_options_from_paths(
    paths: &ProductPaths,
) -> Result<map_viewer::TerrainLodCullOptions> {
    let settings_path = paths.user_data.join("settings.txt");
    let settings = fs::read_to_string(&settings_path).map_err(|error| {
        format!(
            "read selected profile terrain settings {}: {error}",
            settings_path.display()
        )
    })?;
    let options = named_block(&settings, "mapRenderingOptions")?;
    Ok(map_viewer::TerrainLodCullOptions {
        suppress_near_stride_one: yes_no_bool(
            assignment(options, "draw_hires_terrain")?,
            "mapRenderingOptions.draw_hires_terrain",
        )?,
        high_gfx_shaders: yes_no_bool(
            assignment(options, "high_gfx_shaders")?,
            "mapRenderingOptions.high_gfx_shaders",
        )?,
        texture_quality: bounded_u8(
            assignment(options, "texture_quality")?,
            0,
            2,
            "mapRenderingOptions.texture_quality",
        )?,
    })
}

fn source_game_identity(game_install: &Path) -> Result<replay_core::SourceGameIdentity> {
    let launcher_path = game_install.join("launcher-settings.json");
    let launcher: serde_json::Value =
        serde_json::from_slice(&fs::read(&launcher_path).map_err(|error| {
            format!(
                "read the game runtime metadata {}: {error}",
                launcher_path.display()
            )
        })?)
        .map_err(|error| format!("parse {}: {error}", launcher_path.display()))?;
    let full_version = launcher
        .get("gameVersion")
        .or_else(|| launcher.get("version"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            format!(
                "{} has no string gameVersion/version",
                launcher_path.display()
            )
        })?;
    let raw_version = launcher
        .get("rawVersion")
        .and_then(serde_json::Value::as_str);
    let (version, build) = parse_hoi4_version(full_version, raw_version)?;
    let exe_rel = launcher
        .get("exePath")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("{} has no string exePath", launcher_path.display()))?;
    let executable = game_install.join(safe_relative_path(exe_rel)?);
    let executable_bytes = fs::read(&executable).map_err(|error| {
        format!(
            "read the game executable selected by launcher metadata {}: {error}",
            executable.display()
        )
    })?;
    Ok(replay_core::SourceGameIdentity {
        build_profile: format!("hoi4-{version}-{build}"),
        version,
        build,
        executable_sha256: format!("{:x}", Sha256::digest(executable_bytes)),
    })
}

fn parse_hoi4_version(full: &str, raw: Option<&str>) -> Result<(String, String)> {
    let version = [Some(full), raw]
        .into_iter()
        .flatten()
        .find_map(|value| {
            value
                .split(|ch: char| !(ch.is_ascii_digit() || ch == '.'))
                .find(|part| {
                    let pieces = part.split('.').collect::<Vec<_>>();
                    pieces.len() == 4 && pieces.iter().all(|piece| !piece.is_empty())
                })
                .map(str::to_string)
        })
        .ok_or_else(|| format!("launcher metadata has no four-part the game version: {full:?}"))?;
    let build = full
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|part| part.len() == 4)
        .find(|part| {
            part.chars().any(|ch| ch.is_ascii_alphabetic())
                && part.chars().any(|ch| ch.is_ascii_digit())
        })
        .ok_or_else(|| format!("launcher metadata has no Paradox build token: {full:?}"))?
        .to_ascii_lowercase();
    Ok((version, build))
}

fn safe_relative_path(value: &str) -> Result<PathBuf> {
    let normalized = value.trim().replace('\\', "/");
    let normalized = normalized.strip_prefix("./").unwrap_or(&normalized);
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.contains(':')
        || normalized
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(format!("launcher exePath is unsafe/non-relative: {value:?}").into());
    }
    Ok(normalized.split('/').collect())
}

fn exact_material_settings(
    graphics: &str,
    size: &str,
    min_gui: &str,
    settings_bytes: &[u8],
) -> Result<BTreeMap<String, String>> {
    let gui_scale = assignment(graphics, "gui_scale")?
        .parse::<f32>()
        .map_err(|error| format!("parse graphics.gui_scale: {error}"))?;
    if !gui_scale.is_finite() || gui_scale <= 0.0 {
        return Err("graphics.gui_scale must be positive and finite".into());
    }
    let fullscreen = yes_no_bool(assignment(graphics, "fullScreen")?, "fullScreen")?;
    let borderless = yes_no_bool(assignment(graphics, "borderless")?, "borderless")?;
    let display_mode = if fullscreen {
        "fullscreen"
    } else if borderless {
        "borderless_fullscreen"
    } else {
        "windowed"
    };
    Ok(BTreeMap::from([
        ("physical_width".into(), assignment(size, "x")?.into()),
        ("physical_height".into(), assignment(size, "y")?.into()),
        ("min_gui_width".into(), assignment(min_gui, "x")?.into()),
        ("min_gui_height".into(), assignment(min_gui, "y")?.into()),
        ("gui_scale_f32_bits".into(), gui_scale.to_bits().to_string()),
        (
            "refresh_rate_hz".into(),
            assignment(graphics, "refreshRate")?.into(),
        ),
        (
            "max_refresh_rate_hz".into(),
            assignment(graphics, "max_refresh_rate")?.into(),
        ),
        ("fullscreen".into(), fullscreen.to_string()),
        ("borderless".into(), borderless.to_string()),
        ("display_mode".into(), display_mode.into()),
        (
            "settings_file_sha256".into(),
            format!("{:x}", Sha256::digest(settings_bytes)),
        ),
    ]))
}

fn yes_no_bool(value: &str, label: &str) -> Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "yes" | "true" => Ok(true),
        "no" | "false" => Ok(false),
        other => Err(format!("{label} must be yes/no, found {other:?}").into()),
    }
}

fn bounded_u8(value: &str, min: u8, max: u8, label: &str) -> Result<u8> {
    let parsed = value
        .trim()
        .parse::<u8>()
        .map_err(|error| format!("{label} is not an integer: {error}"))?;
    if !(min..=max).contains(&parsed) {
        return Err(format!("{label} must be in {min}..={max}, got {parsed}").into());
    }
    Ok(parsed)
}

fn assignment<'a>(text: &'a str, key: &str) -> Result<&'a str> {
    for line in text.lines() {
        let mut search = 0usize;
        while let Some(relative) = line[search..].find(key) {
            let start = search + relative;
            let before = line[..start].chars().next_back();
            let after_key = start + key.len();
            let after = line[after_key..].chars().next();
            let boundary = |ch: Option<char>| {
                ch.is_none_or(|value| {
                    value.is_whitespace() || matches!(value, '"' | '{' | '}' | '=')
                })
            };
            if boundary(before) && boundary(after) {
                let mut tail = &line[after_key..];
                tail = tail.trim_start_matches(|ch: char| ch.is_whitespace() || ch == '"');
                if let Some(value) = tail.strip_prefix('=') {
                    let value = value.trim_start();
                    if value.starts_with('{') || value.is_empty() {
                        search = after_key;
                        continue;
                    }
                    if let Some(quoted) = value.strip_prefix('"') {
                        let end = quoted.find('"').ok_or_else(|| {
                            format!("unterminated quoted setting value for {key:?}")
                        })?;
                        return Ok(&quoted[..end]);
                    }
                    let end = value
                        .find(|ch: char| ch.is_whitespace() || ch == '}')
                        .unwrap_or(value.len());
                    return Ok(&value[..end]);
                }
            }
            search = after_key;
        }
    }
    Err(format!("missing exact setting {key:?}").into())
}

fn named_block<'a>(text: &'a str, key: &str) -> Result<&'a str> {
    let mut search = 0usize;
    while let Some(relative) = text[search..].find(key) {
        let key_start = search + relative;
        let after = key_start + key.len();
        let tail = &text[after..];
        let Some(eq_rel) = tail.find('=') else { break };
        if tail[..eq_rel]
            .trim_matches(|ch: char| ch.is_whitespace() || ch == '"')
            .is_empty()
        {
            let after_eq = after + eq_rel + 1;
            let Some(open_rel) = text[after_eq..].find('{') else {
                break;
            };
            let open = after_eq + open_rel;
            if !text[after_eq..open].trim().is_empty() {
                search = after;
                continue;
            }
            let mut depth = 0u32;
            let mut quoted = false;
            let mut escaped = false;
            for (offset, ch) in text[open..].char_indices() {
                if escaped {
                    escaped = false;
                    continue;
                }
                if quoted && ch == '\\' {
                    escaped = true;
                    continue;
                }
                if ch == '"' {
                    quoted = !quoted;
                } else if !quoted && ch == '{' {
                    depth += 1;
                } else if !quoted && ch == '}' {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(&text[open + 1..open + offset]);
                    }
                }
            }
            return Err(format!("unterminated setting block {key:?}").into());
        }
        search = after;
    }
    Err(format!("missing setting block {key:?}").into())
}

fn resolve_root<R, W, V>(
    label: &str,
    mut candidate: Option<PathBuf>,
    non_interactive: bool,
    input: &mut R,
    output: &mut W,
    validate: V,
) -> Result<PathBuf>
where
    R: BufRead,
    W: Write,
    V: Fn(&Path) -> std::result::Result<(), String>,
{
    loop {
        if let Some(path) = candidate.take() {
            match validate(&path) {
                Ok(()) => return canonical_root(path),
                Err(error) if non_interactive => {
                    return Err(format!(
                        "invalid {label}: {error}; run replay_app --configure or pass corrected --game/--userdata roots"
                    )
                    .into())
                }
                Err(error) => {
                    writeln!(output, "Saved/selected {label} is invalid: {error}")?;
                }
            }
        }
        if non_interactive {
            return Err(format!(
                "{label} is not configured; run replay_app --configure or pass --game and --userdata"
            )
            .into());
        }
        candidate = Some(prompt_path(label, input, output)?);
    }
}

fn prompt_path<R: BufRead, W: Write>(
    label: &str,
    input: &mut R,
    output: &mut W,
) -> Result<PathBuf> {
    writeln!(output, "First launch/profile selection: enter {label}:")?;
    output.flush()?;
    let mut value = String::new();
    if input.read_line(&mut value)? == 0 {
        return Err(format!(
            "profile selection for {label} received end-of-input; rerun with --configure in a terminal or pass --game/--userdata"
        )
        .into());
    }
    let value = value.trim().trim_matches('"');
    if value.is_empty() {
        return Err(format!("profile selection for {label} was empty").into());
    }
    Ok(PathBuf::from(value))
}

fn canonical_root(path: PathBuf) -> Result<PathBuf> {
    fs::canonicalize(&path).map_err(|error| {
        format!(
            "canonicalize selected directory {}: {error}",
            path.display()
        )
        .into()
    })
}

fn load_with_migration(config: &Path) -> Result<Option<ProductPaths>> {
    if config.is_file() {
        return load_profile(config).map(Some);
    }
    let legacy = legacy_config_path(config);
    if !legacy.is_file() {
        return Ok(None);
    }
    let bytes = fs::read(&legacy)?;
    let paths = serde_json::from_slice::<ProductPaths>(&bytes).map_err(|error| {
        format!(
            "invalid legacy playback profile {}: {error}",
            legacy.display()
        )
    })?;
    // Migration is completed only after normal validation succeeds and the new
    // file is atomically written; the old file is retained for rollback.
    Ok(Some(paths))
}

fn load_profile(config: &Path) -> Result<ProductPaths> {
    let bytes = fs::read(config)?;
    let stored = serde_json::from_slice::<StoredProfile>(&bytes)
        .map_err(|error| format!("invalid playback profile {}: {error}", config.display()))?;
    if stored.schema_version != CONFIG_SCHEMA_VERSION {
        return Err(format!(
            "unsupported playback profile schema_version {} in {}; this build supports {}",
            stored.schema_version,
            config.display(),
            CONFIG_SCHEMA_VERSION
        )
        .into());
    }
    Ok(stored.into())
}

fn store_atomic(config: &Path, paths: &ProductPaths) -> Result<()> {
    let parent = config
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let stored = StoredProfile::from(paths.clone());
    let bytes = serde_json::to_vec_pretty(&stored)?;
    let file_name = config
        .file_name()
        .ok_or_else(|| {
            format!(
                "playback profile path has no file name: {}",
                config.display()
            )
        })?
        .to_string_lossy();
    let (temp, mut file) = (0..64)
        .find_map(|_| {
            let counter = PROFILE_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let candidate = parent.join(format!(
                ".{file_name}.{}.{}.tmp",
                std::process::id(),
                counter
            ));
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&candidate)
            {
                Ok(file) => Some(Ok((candidate, file))),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(error) => Some(Err(error)),
            }
        })
        .transpose()?
        .ok_or("could not allocate playback-profile staging file")?;

    let publish = (|| -> Result<()> {
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        replace_atomic(&temp, config).map_err(|error| {
            format!("commit playback profile {}: {error}", config.display()).into()
        })
    })();
    if publish.is_err() {
        let _ = fs::remove_file(&temp);
    }
    publish
}

#[cfg(windows)]
fn replace_atomic(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
    }

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let replaced = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if replaced == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_atomic(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn root(name: &str) -> PathBuf {
        env::temp_dir().join(format!(
            "replay-app-paths-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn valid_roots(root: &Path) -> (PathBuf, PathBuf) {
        let game = root.join("arbitrary-mounted-game");
        let user = root.join("arbitrary-profile");
        for dir in ["common", "gfx", "map"] {
            fs::create_dir_all(game.join(dir)).unwrap();
        }
        fs::create_dir_all(&user).unwrap();
        fs::write(
            user.join("dlc_load.json"),
            b"{\"enabled_mods\":[],\"disabled_dlcs\":[]}",
        )
        .unwrap();
        (game, user)
    }

    fn valid_profile_roots(root: &Path) -> (PathBuf, PathBuf) {
        let (game, user) = valid_roots(root);
        add_runtime_metadata(&game, &user);
        (game, user)
    }

    fn add_runtime_metadata(game: &Path, user: &Path) {
        fs::write(game.join("hoi4.exe"), b"portable-test-executable").unwrap();
        fs::write(
            game.join("launcher-settings.json"),
            br#"{"gameVersion":"1.19.2.0.a729","rawVersion":"1.19.2.0","exePath":"./hoi4.exe","distPlatform":"steam"}"#,
        )
        .unwrap();
        fs::write(
            user.join("settings.txt"),
            b"language=\"l_english\"\ngraphics={\n size={ x=1920 y=1080 }\n min_gui={ x=1280 y=720 }\n gui_scale=1.000000\n refreshRate=60\n max_refresh_rate=75\n fullScreen=no\n borderless=yes\n}\nmapRenderingOptions={\n draw_terrain=yes\n draw_hires_terrain=no\n high_gfx_shaders=yes\n texture_quality=0\n}\n",
        )
        .unwrap();
        fs::write(
            user.join("pdx_settings.txt"),
            b"\"Graphics\"={}\n\"System\"={\n \"language\"={ value=\"l_english\" version=0 }\n}\n",
        )
        .unwrap();
    }

    #[test]
    fn terrain_lod_option_comes_from_selected_map_rendering_profile() {
        let root = root("terrain-lod-option");
        let (game, user) = valid_profile_roots(&root);
        let paths = ProductPaths {
            game_install: game,
            user_data: user.clone(),
        };
        let initial = playback_terrain_lod_options_from_paths(&paths).unwrap();
        assert!(!initial.suppress_near_stride_one);
        assert!(initial.high_gfx_shaders);
        assert_eq!(initial.texture_quality, 0);
        let settings_path = user.join("settings.txt");
        let settings = fs::read_to_string(&settings_path)
            .unwrap()
            .replace("draw_hires_terrain=no", "draw_hires_terrain=yes")
            .replace("texture_quality=0", "texture_quality=2");
        fs::write(&settings_path, settings).unwrap();
        let changed = playback_terrain_lod_options_from_paths(&paths).unwrap();
        assert!(changed.suppress_near_stride_one);
        assert!(changed.high_gfx_shaders);
        assert_eq!(changed.texture_quality, 2);
        let _ = fs::remove_dir_all(root);
    }

    fn options(game: Option<PathBuf>, user: Option<PathBuf>) -> ResolveOptions {
        ResolveOptions {
            game_override: game,
            user_data_override: user,
            force_reselect: false,
            non_interactive: false,
        }
    }

    #[test]
    fn first_launch_prompts_for_both_paths_and_later_launch_reuses_them() {
        let root = root("prompt");
        let (game, user) = valid_profile_roots(&root);
        let config = root.join("config/profile.json");
        let text = format!("{}\n{}\n", game.display(), user.display());
        let mut input = std::io::Cursor::new(text.into_bytes());
        let mut output = Vec::new();
        let first =
            resolve_or_prompt(&config, ResolveOptions::default(), &mut input, &mut output).unwrap();
        assert_eq!(first.game_install, fs::canonicalize(&game).unwrap());
        assert_eq!(first.user_data, fs::canonicalize(&user).unwrap());
        assert!(String::from_utf8(output)
            .unwrap()
            .contains("First launch/profile selection"));

        let mut no_input = std::io::Cursor::new(Vec::<u8>::new());
        let mut no_output = Vec::new();
        let second = resolve_or_prompt(
            &config,
            ResolveOptions::default(),
            &mut no_input,
            &mut no_output,
        )
        .unwrap();
        assert_eq!(second, first);
        assert!(no_output.is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn invalid_saved_paths_are_reselected_and_explicit_paths_replace_them() {
        let root = root("replace");
        let (game, user) = valid_profile_roots(&root);
        let config = root.join("profile.json");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            &config,
            br#"{"schema_version":1,"game_install":"missing","user_data":"missing"}"#,
        )
        .unwrap();
        let mut input = std::io::Cursor::new(Vec::<u8>::new());
        let mut output = Vec::new();
        let paths = resolve_or_prompt(
            &config,
            options(Some(game.clone()), Some(user.clone())),
            &mut input,
            &mut output,
        )
        .unwrap();
        assert_eq!(paths.game_install, fs::canonicalize(game).unwrap());
        assert_eq!(paths.user_data, fs::canonicalize(user).unwrap());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn profile_replacement_is_complete_and_leaves_no_staging_residue() {
        let root = root("atomic-replace");
        fs::create_dir_all(&root).unwrap();
        let config = root.join(CONFIG_FILE);
        let first = ProductPaths {
            game_install: PathBuf::from("first-game"),
            user_data: PathBuf::from("first-user"),
        };
        let second = ProductPaths {
            game_install: PathBuf::from("second-game"),
            user_data: PathBuf::from("second-user"),
        };
        store_atomic(&config, &first).unwrap();
        store_atomic(&config, &second).unwrap();

        assert_eq!(load_profile(&config).unwrap(), second);
        let residue = fs::read_dir(&root)
            .unwrap()
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect::<Vec<_>>();
        assert!(residue.is_empty(), "staging residue: {residue:?}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn non_interactive_missing_and_invalid_profiles_fail_with_recovery_guidance() {
        let root = root("headless");
        let config = root.join("profile.json");
        let mut input = std::io::Cursor::new(Vec::<u8>::new());
        let mut output = Vec::new();
        let err = resolve_or_prompt(
            &config,
            ResolveOptions {
                non_interactive: true,
                ..ResolveOptions::default()
            },
            &mut input,
            &mut output,
        )
        .unwrap_err();
        assert!(err.to_string().contains("--configure"), "{err}");

        fs::create_dir_all(&root).unwrap();
        fs::write(
            &config,
            br#"{"schema_version":1,"game_install":"missing","user_data":"missing"}"#,
        )
        .unwrap();
        let err = resolve_or_prompt(
            &config,
            ResolveOptions {
                non_interactive: true,
                ..ResolveOptions::default()
            },
            &mut input,
            &mut output,
        )
        .unwrap_err();
        assert!(err.to_string().contains("invalid the game install"), "{err}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn moved_install_and_profile_can_be_reconfigured_without_replay_path_state() {
        let root = root("moved");
        let (old_game, old_user) = valid_profile_roots(&root.join("old"));
        let (new_game, new_user) = valid_profile_roots(&root.join("new"));
        let config = root.join("profile.json");
        let mut input = std::io::Cursor::new(Vec::<u8>::new());
        let mut output = Vec::new();
        resolve_or_prompt(
            &config,
            options(Some(old_game), Some(old_user)),
            &mut input,
            &mut output,
        )
        .unwrap();
        let moved = resolve_or_prompt(
            &config,
            ResolveOptions {
                game_override: Some(new_game.clone()),
                user_data_override: Some(new_user.clone()),
                force_reselect: true,
                non_interactive: true,
            },
            &mut input,
            &mut output,
        )
        .unwrap();
        assert_eq!(moved.game_install, fs::canonicalize(new_game).unwrap());
        assert_eq!(moved.user_data, fs::canonicalize(new_user).unwrap());
        let stored = fs::read_to_string(config).unwrap();
        assert!(!stored.contains("hoi4replay"));
        assert!(!stored.contains("recording"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn legacy_product_paths_migrate_to_versioned_profile() {
        let root = root("migration");
        let (game, user) = valid_profile_roots(&root);
        let config = root.join(CONFIG_FILE);
        let legacy = legacy_config_path(&config);
        fs::write(
            &legacy,
            serde_json::to_vec(&ProductPaths {
                game_install: game,
                user_data: user,
            })
            .unwrap(),
        )
        .unwrap();
        let mut input = std::io::Cursor::new(Vec::<u8>::new());
        let mut output = Vec::new();
        resolve_or_prompt(&config, ResolveOptions::default(), &mut input, &mut output).unwrap();
        let stored: StoredProfile = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        assert_eq!(stored.schema_version, CONFIG_SCHEMA_VERSION);
        assert!(
            legacy.exists(),
            "migration retains legacy file for rollback"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn install_validation_is_content_based_not_executable_or_store_based() {
        let root = root("content-root");
        let (game, _) = valid_roots(&root);
        assert!(!game.join("hoi4.exe").exists());
        validate_game_install(&game).unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn playback_environment_is_derived_from_selected_roots_without_manifest_defaults() {
        let root = root("environment");
        let (game, user) = valid_roots(&root);
        add_runtime_metadata(&game, &user);
        let resolver = map_viewer::GameFileResolver::from_dlc_load(&game, user.clone()).unwrap();
        let keys = [
            "physical_width",
            "physical_height",
            "min_gui_width",
            "min_gui_height",
            "gui_scale_f32_bits",
            "refresh_rate_hz",
            "max_refresh_rate_hz",
            "fullscreen",
            "borderless",
            "display_mode",
            "settings_file_sha256",
        ]
        .into_iter()
        .map(str::to_string);
        let environment = playback_environment_from_paths(
            &ProductPaths {
                game_install: game,
                user_data: user,
            },
            &resolver,
            keys,
        )
        .unwrap();
        assert_eq!(environment.source_game.version, "1.19.2.0");
        assert_eq!(environment.source_game.build, "a729");
        assert_eq!(environment.source_game.build_profile, "hoi4-1.19.2.0-a729");
        assert_eq!(environment.localisation_language, "l_english");
        assert_eq!(environment.settings["physical_width"], "1920");
        assert_eq!(
            environment.settings["display_mode"],
            "borderless_fullscreen"
        );
        assert_eq!(environment.settings["gui_scale_f32_bits"], "1065353216");
        assert_eq!(environment.source_game.executable_sha256.len(), 64);
        assert!(!environment.source_profile.profile_id.contains("\\"));
        assert!(environment
            .source_profile
            .profile_id
            .starts_with("runtime-profile:"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn unknown_material_setting_fails_instead_of_copying_replay_value() {
        let root = root("unknown-setting");
        let (game, user) = valid_roots(&root);
        add_runtime_metadata(&game, &user);
        let resolver = map_viewer::GameFileResolver::from_dlc_load(&game, user.clone()).unwrap();
        let err = playback_environment_from_paths(
            &ProductPaths {
                game_install: game,
                user_data: user,
            },
            &resolver,
            ["future_unproven_setting".to_string()],
        )
        .unwrap_err();
        assert!(err.to_string().contains("no proven parser"), "{err}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn full_validation_failure_does_not_replace_last_good_profile() {
        let root = root("reject-before-store");
        let (good_game, good_user) = valid_profile_roots(&root.join("good"));
        let config = root.join(CONFIG_FILE);
        let mut input = std::io::Cursor::new(Vec::<u8>::new());
        let mut output = Vec::new();
        let good = resolve_or_prompt(
            &config,
            ResolveOptions {
                game_override: Some(good_game),
                user_data_override: Some(good_user),
                force_reselect: true,
                non_interactive: true,
            },
            &mut input,
            &mut output,
        )
        .unwrap();
        let committed = fs::read(&config).unwrap();

        let (bad_game, bad_user) = valid_profile_roots(&root.join("bad"));
        fs::write(bad_user.join("dlc_load.json"), b"not valid json").unwrap();
        let error = resolve_or_prompt(
            &config,
            ResolveOptions {
                game_override: Some(bad_game),
                user_data_override: Some(bad_user),
                force_reselect: true,
                non_interactive: true,
            },
            &mut input,
            &mut output,
        )
        .expect_err("malformed launcher selection must fail before persistence");
        assert!(error.to_string().contains("launcher"), "{error}");
        assert_eq!(fs::read(&config).unwrap(), committed);
        assert_eq!(load_profile(&config).unwrap(), good);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn saved_profile_is_fully_revalidated_before_reuse() {
        let root = root("revalidate-saved");
        let (game, user) = valid_profile_roots(&root);
        let config = root.join(CONFIG_FILE);
        store_atomic(
            &config,
            &ProductPaths {
                game_install: fs::canonicalize(game).unwrap(),
                user_data: fs::canonicalize(&user).unwrap(),
            },
        )
        .unwrap();
        let committed = fs::read(&config).unwrap();
        fs::remove_file(user.join("settings.txt")).unwrap();

        let mut input = std::io::Cursor::new(Vec::<u8>::new());
        let mut output = Vec::new();
        let error = resolve_or_prompt(
            &config,
            ResolveOptions {
                non_interactive: true,
                ..ResolveOptions::default()
            },
            &mut input,
            &mut output,
        )
        .expect_err("saved roots with missing metadata must not be reused");
        assert!(error.to_string().contains("settings.txt"), "{error}");
        assert_eq!(fs::read(&config).unwrap(), committed);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn machine_local_product_paths_share_the_local_app_data_root() {
        let local = Path::new("C:/Users/Example/AppData/Local");
        assert_eq!(
            config_path_from_local_app_data(local),
            local.join("battle-plan").join("playback_profile.json")
        );
        assert_eq!(
            recorder_recordings_root_from_local_app_data(local),
            local.join("Battle Plan Recorder")
        );
    }

    #[test]
    fn present_unreadable_optional_profile_file_never_degrades_to_absent() {
        let root = root("unreadable-optional-profile-file");
        let (game, user) = valid_roots(&root);
        add_runtime_metadata(&game, &user);
        fs::remove_file(user.join("pdx_settings.txt")).unwrap();
        fs::create_dir(user.join("pdx_settings.txt")).unwrap();
        let resolver = map_viewer::GameFileResolver::from_dlc_load(&game, user.clone()).unwrap();

        let error = playback_environment_from_paths(
            &ProductPaths {
                game_install: game,
                user_data: user,
            },
            &resolver,
            std::iter::empty(),
        )
        .expect_err("present unreadable pdx_settings.txt must not use settings.txt fallback");
        let message = error.to_string();
        assert!(message.contains("pdx_settings.txt"), "{message}");
        assert!(message.contains("present but unreadable"), "{message}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn dlc_selection_uses_shared_short_identity_not_descriptor_path() {
        assert_eq!(
            replay_core::normalize_dlc_selection_id("dlc/dlc020/dlc020.dlc").unwrap(),
            "dlc020"
        );
        assert_eq!(
            replay_core::normalize_dlc_selection_id("dlc/dlc020_la_resistance/dlc020.dlc").unwrap(),
            "dlc020_la_resistance"
        );
        let err =
            replay_core::normalize_dlc_selection_id("C:/machine/path/dlc020.dlc").unwrap_err();
        assert!(err.contains("unsafe/non-canonical"), "{err}");
    }
}
