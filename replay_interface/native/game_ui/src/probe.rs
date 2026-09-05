use crate::assets::{normalize_rel_path, AssetResolver};
use crate::dds;
use crate::font::BitmapFont;
use crate::gfx::{GfxParseStats, SpriteRegistry};
use crate::gui::{GuiParseStats, LayoutSet};
use crate::hud::{
    compose_hud, inspect_hud_sprites, load_hud_layouts, write_hud_proofs, HudBinding,
};
use crate::loc::Localisation;
use crate::major_views::{inspect_major_views, write_major_view_proofs, MajorViewReport};
use crate::paradox_text::{destructive_event_count, diagnose_text_decode, parse_bytes, parse_checked_bytes};
use crate::png::write_png;
use crate::tga;
use crate::topbar::{
    binding_fields, compose_topbar, load_topbar_layout, topbar_structure_summary,
    write_topbar_proofs, TopbarBinding,
};
use crate::{ErrorKind, GameUiError, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub const DEFAULT_GAME_ROOT: &str = r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeConfig {
    pub vanilla_root: PathBuf,
    pub mod_root: Option<PathBuf>,
    pub out_dir: PathBuf,
    pub write_pngs: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopbarDecodeReport {
    pub sprite_ref_count: usize,
    pub resolved_texture_count: usize,
    pub decoded_texture_count: usize,
    pub missing: Vec<String>,
    pub undecodable: Vec<String>,
    pub dds_format_histogram: BTreeMap<String, usize>,
    pub texture_fallbacks: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopbarWindowReport {
    pub source_file: String,
    pub window_name: String,
    pub viewport_width: usize,
    pub viewport_height: usize,
    pub element_histogram: BTreeMap<String, usize>,
    pub binding_fields: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopbarCompositionReport {
    pub sample_drawn_elements: usize,
    pub sample_missing_sprites: usize,
    pub sample_undecodable_sprites: usize,
    pub sample_missing_glyphs: BTreeMap<String, String>,
    pub sample_unresolved_loc_keys: Vec<String>,
    pub sample_flag_texture: Option<String>,
    pub empty_drawn_elements: usize,
    pub empty_missing_sprites: usize,
    pub empty_undecodable_sprites: usize,
    pub empty_missing_glyphs: BTreeMap<String, String>,
    pub empty_unresolved_loc_keys: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HudWindowReport {
    pub source_windows: Vec<String>,
    pub referenced_sprites: usize,
    pub decoded_sprites: usize,
    pub missing_sprites: Vec<String>,
    pub undecodable_sprites: Vec<String>,
    pub tooltip_source_note: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HudCompositionReport {
    pub sample_drawn_elements: usize,
    pub sample_missing_sprites: usize,
    pub sample_undecodable_sprites: usize,
    pub sample_missing_glyphs: BTreeMap<String, String>,
    pub sample_unresolved_loc_keys: Vec<String>,
    pub empty_drawn_elements: usize,
    pub empty_missing_sprites: usize,
    pub empty_undecodable_sprites: usize,
    pub empty_missing_glyphs: BTreeMap<String, String>,
    pub empty_unresolved_loc_keys: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalisationProbeReport {
    pub file_count: usize,
    pub key_count: usize,
    pub parse_failures: Vec<String>,
    pub decode_lossy_files: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontProbeReport {
    pub checked_fonts: Vec<String>,
    pub missing_ascii: BTreeMap<String, String>,
    pub missing_topbar_sample_glyphs: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReferencedDdsReport {
    pub texture_count: usize,
    pub format_histogram: BTreeMap<String, usize>,
    pub unsupported_histogram: BTreeMap<String, usize>,
    pub unreadable: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeReport {
    pub roots: Vec<PathBuf>,
    pub gfx: GfxParseStats,
    pub gui: GuiParseStats,
    pub topbar: TopbarDecodeReport,
    pub topbar_window: TopbarWindowReport,
    pub topbar_composition: TopbarCompositionReport,
    pub localisation: LocalisationProbeReport,
    pub hud_window: HudWindowReport,
    pub hud_composition: HudCompositionReport,
    pub major_views: Vec<MajorViewReport>,
    pub fonts: FontProbeReport,
    pub referenced_dds: ReferencedDdsReport,
    pub png_dumps: Vec<PathBuf>,
    pub parser_notes: Vec<String>,
}

pub fn default_probe_config() -> ProbeConfig {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .ancestors()
        .nth(3)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| manifest_dir.clone());
    ProbeConfig {
        vanilla_root: std::env::var_os("BATTLE_PLAN_GAME_ROOT").or_else(|| std::env::var_os("HOI4_GAME_ROOT"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_GAME_ROOT)),
        mod_root: Some(
            std::env::var_os("GAME_UI_MOD_ROOT")
                .map(PathBuf::from)
                .unwrap_or(repo_root),
        ),
        out_dir: manifest_dir.join("out"),
        write_pngs: true,
    }
}

pub fn run_probe(config: &ProbeConfig) -> Result<ProbeReport> {
    if !config.vanilla_root.is_dir() {
        return Err(GameUiError::missing(format!(
            "game root is missing: {}",
            config.vanilla_root.display()
        )));
    }
    let mut roots = Vec::new();
    if let Some(mod_root) = &config.mod_root {
        if mod_root.is_dir() {
            roots.push(mod_root.clone());
        }
    }
    roots.push(config.vanilla_root.clone());
    let resolver = AssetResolver::new(roots.clone());

    let (registry, gfx) = parse_gfx_files(&resolver)?;
    let (_, gui) = parse_gui_files(&resolver)?;
    let topbar = inspect_topbar(&resolver, &registry)?;
    let topbar_window = inspect_topbar_window(&resolver, &registry)?;
    let topbar_composition = inspect_topbar_composition(&resolver, &registry)?;
    let localisation = inspect_localisation(&resolver)?;
    let hud_window = inspect_hud_window(&resolver, &registry)?;
    let hud_composition = inspect_hud_composition(&resolver, &registry)?;
    let mut major_views = inspect_major_views(&resolver, &registry)?;
    let fonts = inspect_fonts(&resolver)?;
    let referenced_dds = scan_referenced_dds(&resolver, &registry);
    let png_dumps = if config.write_pngs {
        let mut paths = write_probe_pngs(&resolver, &registry, &config.out_dir)?;
        for (key, path) in write_major_view_proofs(&resolver, &registry, &config.out_dir)? {
            if let Some(report) = major_views.iter_mut().find(|report| report.key == key) {
                report.proof_png = Some(path.clone());
            }
            paths.push(path);
        }
        paths
    } else {
        Vec::new()
    };
    Ok(ProbeReport {
        roots,
        gfx,
        gui,
        topbar,
        topbar_window,
        topbar_composition,
        localisation,
        hud_window,
        hud_composition,
        major_views,
        fonts,
        referenced_dds,
        png_dumps,
        parser_notes: parser_notes(),
    })
}

fn load_gfx_document(resolver: &AssetResolver, file: &crate::assets::ResolvedFile) -> Result<(crate::paradox_text::Document, usize)> {
    let bytes = resolver.read_file(file)?;
    let events = destructive_event_count(&diagnose_text_decode(&bytes));
    Ok((parse_bytes(&bytes), events))
}

pub fn parse_gfx_files(resolver: &AssetResolver) -> Result<(SpriteRegistry, GfxParseStats)> {
    let files = resolver.list_files_recursive("interface", Some("gfx"))?;
    let mut registry = SpriteRegistry::new();
    let mut stats = GfxParseStats {
        file_count: files.len(),
        ..GfxParseStats::default()
    };
    for file in files {
        match load_gfx_document(resolver, &file) {
            Ok((doc, decode_events)) => {
                stats.recovery_count += doc.recoveries.len();
                if !doc.recoveries.is_empty() {
                    stats.files_with_recoveries.push(file.rel_path.clone());
                }
                if decode_events != 0 {
                    stats.decode_loss_count += decode_events;
                    stats.files_with_decode_loss.push(file.rel_path.clone());
                }
                registry.add_document(&file.rel_path, &doc);
            }
            Err(err) => {
                stats.parse_failures += 1;
                stats
                    .failures
                    .push(format!("{}: {}", file.rel_path, err.message));
            }
        }
    }
    stats.sprite_count = registry.len();
    stats.kind_histogram = registry.kind_histogram();
    Ok((registry, stats))
}

pub fn parse_gui_files(resolver: &AssetResolver) -> Result<(LayoutSet, GuiParseStats)> {
    let files = resolver.list_files_recursive("interface", Some("gui"))?;
    let mut layouts = LayoutSet::default();
    for file in files {
        match layouts.parse_content(&resolver.read_file(&file)?, &file.rel_path) {
            Ok(()) => {}
            Err(err) => {
                layouts.stats.file_count += 1;
                layouts.stats.parse_failures += 1;
                layouts
                    .stats
                    .failures
                    .push(format!("{}: {}", file.rel_path, err.message));
            }
        }
    }
    let stats = layouts.stats.clone();
    Ok((layouts, stats))
}

fn inspect_topbar(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
) -> Result<TopbarDecodeReport> {
    let topbar = resolver.resolve_file("interface/topbar.gui")?;
    let doc = parse_checked_bytes(&resolver.read_file(&topbar)?, &topbar.rel_path)?;
    let mut layouts = LayoutSet::default();
    layouts.add_document(&topbar.rel_path, &doc);
    let sprite_refs = layouts.sprite_refs();
    let mut report = TopbarDecodeReport {
        sprite_ref_count: sprite_refs.len(),
        resolved_texture_count: 0,
        decoded_texture_count: 0,
        missing: Vec::new(),
        undecodable: Vec::new(),
        dds_format_histogram: BTreeMap::new(),
        texture_fallbacks: Vec::new(),
    };

    for sprite_name in sprite_refs {
        let Some(sprite) = registry.get(&sprite_name) else {
            report
                .missing
                .push(format!("{sprite_name}: sprite registry entry missing"));
            continue;
        };
        if sprite.texture_files.is_empty() {
            report
                .undecodable
                .push(format!("{sprite_name}: sprite has no texturefile"));
            continue;
        }
        for texture in &sprite.texture_files {
            let rel = normalize_rel_path(&texture.path);
            let resolved = match resolver.resolve_texture_file(&rel) {
                Ok(resolved) => resolved,
                Err(_) => {
                    report
                        .missing
                        .push(format!("{sprite_name}: missing texture {rel}"));
                    continue;
                }
            };
            if normalize_rel_path(&resolved.rel_path) != rel {
                report
                    .texture_fallbacks
                    .push(format!("{sprite_name}: {} -> {}", rel, resolved.rel_path));
            }
            report.resolved_texture_count += 1;
            let ext = resolved
                .path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            match ext.as_str() {
                "dds" => {
                    if let Ok(info) = resolver.read_file(&resolved).and_then(|bytes| dds::info_bytes(&bytes)) {
                        *report.dds_format_histogram.entry(info.format).or_insert(0) += 1;
                    }
                    match resolver.read_file(&resolved).and_then(|bytes| dds::decode_bytes(&bytes)) {
                        Ok(_) => report.decoded_texture_count += 1,
                        Err(err) => report.undecodable.push(format!(
                            "{sprite_name}: {} decode failed: {}",
                            resolved.rel_path, err.message
                        )),
                    }
                }
                "tga" => match resolver.read_file(&resolved).and_then(|bytes| tga::decode_bytes(&bytes)) {
                    Ok(decoded) => {
                        *report
                            .dds_format_histogram
                            .entry(decoded.format)
                            .or_insert(0) += 1;
                        report.decoded_texture_count += 1;
                    }
                    Err(err) => report.undecodable.push(format!(
                        "{sprite_name}: {} decode failed: {}",
                        resolved.rel_path, err.message
                    )),
                },
                _ => report.undecodable.push(format!(
                    "{sprite_name}: {} uses unsupported extension .{}",
                    resolved.rel_path,
                    if ext.is_empty() {
                        "<none>"
                    } else {
                        ext.as_str()
                    }
                )),
            }
        }
    }

    report.missing.sort();
    report.undecodable.sort();
    report.texture_fallbacks.sort();
    Ok(report)
}

fn inspect_topbar_window(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
) -> Result<TopbarWindowReport> {
    let layout = load_topbar_layout(resolver, registry)?;
    let element_histogram = topbar_structure_summary(&layout);
    Ok(TopbarWindowReport {
        source_file: layout.source_file,
        window_name: layout.window_name,
        viewport_width: layout.viewport_width,
        viewport_height: layout.viewport_height,
        element_histogram,
        binding_fields: binding_fields()
            .into_iter()
            .map(|field| {
                format!(
                    "{} -> {} (default {}){}",
                    field.field,
                    field.gui_element,
                    field.default,
                    if field.notes.is_empty() {
                        String::new()
                    } else {
                        format!("; {}", field.notes)
                    }
                )
            })
            .collect(),
    })
}

fn inspect_topbar_composition(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
) -> Result<TopbarCompositionReport> {
    let sample = compose_topbar(resolver, registry, &TopbarBinding::sample_ire())?;
    let empty = compose_topbar(resolver, registry, &TopbarBinding::default())?;
    Ok(TopbarCompositionReport {
        sample_drawn_elements: sample.output.report.drawn_elements,
        sample_missing_sprites: sample.output.report.missing_sprites.len(),
        sample_undecodable_sprites: sample.output.report.undecodable_sprites.len(),
        sample_missing_glyphs: stringify_missing_glyphs(&sample.output.report.missing_glyphs),
        sample_unresolved_loc_keys: stringify_unresolved_loc_keys(
            &sample.output.report.localisation_misses,
        ),
        sample_flag_texture: sample.flag_texture_used,
        empty_drawn_elements: empty.output.report.drawn_elements,
        empty_missing_sprites: empty.output.report.missing_sprites.len(),
        empty_undecodable_sprites: empty.output.report.undecodable_sprites.len(),
        empty_missing_glyphs: stringify_missing_glyphs(&empty.output.report.missing_glyphs),
        empty_unresolved_loc_keys: stringify_unresolved_loc_keys(
            &empty.output.report.localisation_misses,
        ),
    })
}

fn inspect_localisation(resolver: &AssetResolver) -> Result<LocalisationProbeReport> {
    let localisation = Localisation::load_english(resolver)?;
    Ok(LocalisationProbeReport {
        file_count: localisation.source_files.len(),
        key_count: localisation.entries.len(),
        parse_failures: localisation.parse_failures,
        decode_lossy_files: localisation.decode_lossy_files,
    })
}

fn inspect_hud_window(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
) -> Result<HudWindowReport> {
    let layouts = load_hud_layouts(resolver)?;
    let stats = inspect_hud_sprites(resolver, registry)?;
    Ok(HudWindowReport {
        source_windows: layouts
            .sources
            .into_iter()
            .map(|source| {
                format!(
                    "{}: {} window={}",
                    source.panel, source.source_file, source.window_name
                )
            })
            .collect(),
        referenced_sprites: stats.referenced,
        decoded_sprites: stats.decoded,
        missing_sprites: stats.missing,
        undecodable_sprites: stats.undecodable,
        tooltip_source_note: "Vanilla generic tooltip chrome is interface/core.gui window=ToolTip, backed by textureFile gfx\\\\interface\\\\tiles\\\\tiles_dialog.tga and font cg_16b."
            .to_string(),
    })
}

fn inspect_hud_composition(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
) -> Result<HudCompositionReport> {
    let sample = compose_hud(resolver, registry, &HudBinding::sample(), 1920, 1080)?;
    let empty = compose_hud(resolver, registry, &HudBinding::default(), 1920, 1080)?;
    Ok(HudCompositionReport {
        sample_drawn_elements: sample.output.report.drawn_elements,
        sample_missing_sprites: sample.output.report.missing_sprites.len()
            + sample.sprite_stats.missing.len(),
        sample_undecodable_sprites: sample.output.report.undecodable_sprites.len()
            + sample.sprite_stats.undecodable.len(),
        sample_missing_glyphs: stringify_missing_glyphs(&sample.output.report.missing_glyphs),
        sample_unresolved_loc_keys: stringify_unresolved_loc_keys(
            &sample.output.report.localisation_misses,
        ),
        empty_drawn_elements: empty.output.report.drawn_elements,
        empty_missing_sprites: empty.output.report.missing_sprites.len()
            + empty.sprite_stats.missing.len(),
        empty_undecodable_sprites: empty.output.report.undecodable_sprites.len()
            + empty.sprite_stats.undecodable.len(),
        empty_missing_glyphs: stringify_missing_glyphs(&empty.output.report.missing_glyphs),
        empty_unresolved_loc_keys: stringify_unresolved_loc_keys(
            &empty.output.report.localisation_misses,
        ),
    })
}

fn stringify_missing_glyphs(
    missing: &BTreeMap<String, BTreeSet<char>>,
) -> BTreeMap<String, String> {
    missing
        .iter()
        .map(|(key, chars)| (key.clone(), chars.iter().collect()))
        .collect()
}

fn stringify_unresolved_loc_keys(misses: &replay_core::LocalisationMisses) -> Vec<String> {
    misses.keys().map(str::to_string).collect()
}

fn inspect_fonts(resolver: &AssetResolver) -> Result<FontProbeReport> {
    let sample = "9 Nov 1943, 17:00 0% 100% 12/24/6 216 91d !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~";
    let mut report = FontProbeReport {
        checked_fonts: Vec::new(),
        missing_ascii: BTreeMap::new(),
        missing_topbar_sample_glyphs: BTreeMap::new(),
    };
    for font_name in ["hoi_16mbs", "hoi_18mbs", "hoi_26mbs"] {
        let font = BitmapFont::load(resolver, font_name)?;
        report.checked_fonts.push(font_name.to_string());
        let missing_ascii = (32u8..=126)
            .filter(|byte| !font.chars.contains_key(&u32::from(*byte)))
            .map(char::from)
            .collect::<String>();
        if !missing_ascii.is_empty() {
            report
                .missing_ascii
                .insert(font_name.to_string(), missing_ascii);
        }
        let missing_sample = font.missing_chars(sample).iter().collect::<String>();
        if !missing_sample.is_empty() {
            report
                .missing_topbar_sample_glyphs
                .insert(font_name.to_string(), missing_sample);
        }
    }
    Ok(report)
}

fn scan_referenced_dds(resolver: &AssetResolver, registry: &SpriteRegistry) -> ReferencedDdsReport {
    let mut seen = BTreeSet::new();
    let mut report = ReferencedDdsReport {
        texture_count: 0,
        format_histogram: BTreeMap::new(),
        unsupported_histogram: BTreeMap::new(),
        unreadable: Vec::new(),
    };
    for sprite in registry.sprites() {
        for texture in &sprite.texture_files {
            let rel = normalize_rel_path(&texture.path);
            if !seen.insert(rel.clone()) {
                continue;
            }
            let ext = Path::new(&rel)
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if ext != "dds" {
                continue;
            }
            let resolved = match resolver.resolve_optional_file(&rel) {
                Ok(Some(resolved)) => resolved,
                Ok(None) => {
                    report.unreadable.push(format!("{rel}: missing"));
                    continue;
                }
                Err(err) => {
                    report.unreadable.push(format!("{rel}: {}", err.message));
                    continue;
                }
            };
            match resolver.read_file(&resolved).and_then(|bytes| dds::info_bytes(&bytes)) {
                Ok(info) => {
                    report.texture_count += 1;
                    *report
                        .format_histogram
                        .entry(info.format.clone())
                        .or_insert(0) += 1;
                    if !is_supported_dds_format(&info.format) {
                        *report.unsupported_histogram.entry(info.format).or_insert(0) += 1;
                    }
                }
                Err(err) => match err.kind {
                    ErrorKind::UnsupportedDdsFormat(format) => {
                        report.texture_count += 1;
                        *report.format_histogram.entry(format.clone()).or_insert(0) += 1;
                        *report.unsupported_histogram.entry(format).or_insert(0) += 1;
                    }
                    _ => report.unreadable.push(format!("{rel}: {}", err.message)),
                },
            }
        }
    }
    report.unreadable.sort();
    report
}

fn write_probe_pngs(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    out_dir: &Path,
) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for (sprite_name, file_name) in [
        (
            "GFX_topbar_background_extended",
            "topbar_background_extended.png",
        ),
        ("GFX_resources_strip", "resources_strip.png"),
        ("GFX_speed_step", "speed_step.png"),
        (
            "GFX_world_tension_icon_big_strip",
            "world_tension_icon_big_strip.png",
        ),
        ("GFX_WT_anim_strip", "world_tension_fire_strip.png"),
        ("GFX_armyoverview_button", "armyoverview_button_sheet.png"),
        ("GFX_navyoverview_button", "navyoverview_button_sheet.png"),
        ("GFX_airoverview_button", "airoverview_button_sheet.png"),
    ] {
        let sprite = registry
            .get(sprite_name)
            .ok_or_else(|| GameUiError::missing(format!("dump sprite missing: {sprite_name}")))?;
        let decoded = resolver.decode_sprite(sprite)?;
        let path = out_dir.join(file_name);
        write_png(&path, &decoded.texture.image)?;
        paths.push(path);
    }

    let sprite_name = "GFX_decisions_glow";
    let sprite = registry
        .get(sprite_name)
        .ok_or_else(|| GameUiError::missing(format!("dump sprite missing: {sprite_name}")))?;
    let decoded = resolver.decode_sprite(sprite)?;
    for (idx, frame) in decoded.frames.iter().enumerate() {
        let path = out_dir.join(format!("decisions_glow_frame_{idx:02}.png"));
        write_png(&path, frame)?;
        paths.push(path);
    }
    for (rel, file_name) in [
        ("gfx/flags/IRE_neutrality.tga", "ire_flag_neutrality.png"),
        ("gfx/flags/IRE_democratic.tga", "ire_flag_democratic.png"),
        ("gfx/flags/small/IRE.tga", "ire_flag_plain_small.png"),
    ] {
        if resolver.resolve_optional_file(rel)?.is_some() {
            let decoded = resolver.decode_texture(rel)?;
            let path = out_dir.join(file_name);
            write_png(&path, &decoded.image)?;
            paths.push(path);
        }
    }
    paths.extend(write_topbar_proofs(resolver, registry, out_dir)?);
    paths.extend(write_hud_proofs(resolver, registry, out_dir)?);
    Ok(paths)
}

fn is_supported_dds_format(format: &str) -> bool {
    matches!(
        format,
        "DXT1" | "DXT3" | "DXT5" | "A8R8G8B8" | "X8R8G8B8" | "R8G8B8"
    )
}

fn parser_notes() -> Vec<String> {
    vec![
        "Decoding is UTF-8 first with UTF-8/UTF-16 BOM stripping and latin-1 fallback.".to_string(),
        "Repeated keys are preserved in source order; spriteType and GUI child blocks rely on this.".to_string(),
        "Brace blocks allow both key/value pairs and bare items, needed for color arrays like { 0.0 0.5 0.0 }.".to_string(),
        "Observed real-file quirks include doubled percent atoms such as 100%%, mixed-case keys such as SpriteType/Orientation/textureFile, negative anchored positions, comments after values, and generic child blocks such as background.".to_string(),
        "the game text color escapes using §G/§R/§Y/§! are parsed into per-glyph tint runs before bitmap-font rendering.".to_string(),
    ]
}

impl ProbeReport {
    pub fn unresolved_loc_keys(&self) -> BTreeSet<String> {
        let mut keys = BTreeSet::new();
        keys.extend(
            self.topbar_composition
                .sample_unresolved_loc_keys
                .iter()
                .cloned(),
        );
        keys.extend(
            self.topbar_composition
                .empty_unresolved_loc_keys
                .iter()
                .cloned(),
        );
        keys.extend(
            self.hud_composition
                .sample_unresolved_loc_keys
                .iter()
                .cloned(),
        );
        keys.extend(
            self.hud_composition
                .empty_unresolved_loc_keys
                .iter()
                .cloned(),
        );
        for view in &self.major_views {
            keys.extend(view.sample_unresolved_loc_keys.iter().cloned());
        }
        keys
    }

    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str("game_ui probe report\n");
        out.push_str("roots:\n");
        for root in &self.roots {
            out.push_str(&format!("  {}\n", root.display()));
        }
        out.push_str(&format!(
            "gfx files={} sprites={} parse_failures={} recoveries={}\n",
            self.gfx.file_count,
            self.gfx.sprite_count,
            self.gfx.parse_failures,
            self.gfx.recovery_count
        ));
        out.push_str("gfx sprite kinds:\n");
        push_histogram(&mut out, &self.gfx.kind_histogram);
        if !self.gfx.files_with_decode_loss.is_empty() {
            out.push_str(&format!(
                "gfx files with destructive decode loss (events={}):\n",
                self.gfx.decode_loss_count
            ));
            for file in &self.gfx.files_with_decode_loss {
                out.push_str(&format!("  {file}\n"));
            }
        }
        if !self.gfx.files_with_recoveries.is_empty() {
            out.push_str("gfx files with parser recoveries:\n");
            for file in &self.gfx.files_with_recoveries {
                out.push_str(&format!("  {file}\n"));
            }
        }
        out.push_str(&format!(
            "gui files={} windows={} elements={} parse_failures={} recoveries={}\n",
            self.gui.file_count,
            self.gui.window_count,
            self.gui.element_count,
            self.gui.parse_failures,
            self.gui.recovery_count
        ));
        out.push_str("gui unknown element histogram:\n");
        push_histogram(&mut out, &self.gui.unknown_element_histogram);
        if !self.gui.files_with_recoveries.is_empty() {
            out.push_str("gui files with parser recoveries:\n");
            for file in &self.gui.files_with_recoveries {
                out.push_str(&format!("  {file}\n"));
            }
        }
        if !self.gui.files_with_decode_loss.is_empty() {
            out.push_str(&format!(
                "gui files with destructive decode loss (events={}):\n",
                self.gui.decode_loss_count
            ));
            for file in &self.gui.files_with_decode_loss {
                out.push_str(&format!("  {file}\n"));
            }
        }
        out.push_str(&format!(
            "localisation english files={} keys={} parse_failures={}\n",
            self.localisation.file_count,
            self.localisation.key_count,
            self.localisation.parse_failures.len()
        ));
        if !self.localisation.parse_failures.is_empty() {
            out.push_str("localisation parse failures:\n");
            for item in &self.localisation.parse_failures {
                out.push_str(&format!("  {item}\n"));
            }
        }
        if !self.localisation.decode_lossy_files.is_empty() {
            out.push_str("localisation files with destructive decode loss:\n");
            for item in &self.localisation.decode_lossy_files {
                out.push_str(&format!("  {item}\n"));
            }
        }
        let unresolved_loc_keys = self.unresolved_loc_keys();
        out.push_str("unresolved loc keys:\n");
        if unresolved_loc_keys.is_empty() {
            out.push_str("  <none>\n");
        } else {
            for key in unresolved_loc_keys {
                out.push_str(&format!("  {key}\n"));
            }
        }
        out.push_str(&format!(
            "topbar sprite_refs={} resolved_textures={} decoded_textures={} missing={} undecodable={}\n",
            self.topbar.sprite_ref_count,
            self.topbar.resolved_texture_count,
            self.topbar.decoded_texture_count,
            self.topbar.missing.len(),
            self.topbar.undecodable.len()
        ));
        out.push_str("topbar DDS formats:\n");
        push_histogram(&mut out, &self.topbar.dds_format_histogram);
        if !self.topbar.texture_fallbacks.is_empty() {
            out.push_str("topbar texture fallbacks:\n");
            for item in &self.topbar.texture_fallbacks {
                out.push_str(&format!("  {item}\n"));
            }
        }
        if !self.topbar.missing.is_empty() {
            out.push_str("topbar missing:\n");
            for item in &self.topbar.missing {
                out.push_str(&format!("  {item}\n"));
            }
        }
        if !self.topbar.undecodable.is_empty() {
            out.push_str("topbar undecodable:\n");
            for item in &self.topbar.undecodable {
                out.push_str(&format!("  {item}\n"));
            }
        }
        out.push_str(&format!(
            "topbar window: {} window={} viewport={}x{}\n",
            self.topbar_window.source_file,
            self.topbar_window.window_name,
            self.topbar_window.viewport_width,
            self.topbar_window.viewport_height
        ));
        out.push_str("topbar structure histogram:\n");
        push_histogram(&mut out, &self.topbar_window.element_histogram);
        out.push_str("topbar binding fields:\n");
        for field in &self.topbar_window.binding_fields {
            out.push_str(&format!("  {field}\n"));
        }
        out.push_str(&format!(
            "topbar composition sample: drawn={} missing_sprites={} undecodable_sprites={} missing_glyph_sets={} flag={}\n",
            self.topbar_composition.sample_drawn_elements,
            self.topbar_composition.sample_missing_sprites,
            self.topbar_composition.sample_undecodable_sprites,
            self.topbar_composition.sample_missing_glyphs.len(),
            self.topbar_composition
                .sample_flag_texture
                .as_deref()
                .unwrap_or("<none>")
        ));
        if !self.topbar_composition.sample_missing_glyphs.is_empty() {
            out.push_str("topbar composition sample missing glyphs:\n");
            for (element, glyphs) in &self.topbar_composition.sample_missing_glyphs {
                out.push_str(&format!("  {element}: {glyphs}\n"));
            }
        }
        if !self
            .topbar_composition
            .sample_unresolved_loc_keys
            .is_empty()
        {
            out.push_str("topbar composition sample unresolved loc keys:\n");
            for key in &self.topbar_composition.sample_unresolved_loc_keys {
                out.push_str(&format!("  {key}\n"));
            }
        }
        out.push_str(&format!(
            "topbar composition empty: drawn={} missing_sprites={} undecodable_sprites={} missing_glyph_sets={}\n",
            self.topbar_composition.empty_drawn_elements,
            self.topbar_composition.empty_missing_sprites,
            self.topbar_composition.empty_undecodable_sprites,
            self.topbar_composition.empty_missing_glyphs.len()
        ));
        if !self.topbar_composition.empty_missing_glyphs.is_empty() {
            out.push_str("topbar composition empty missing glyphs:\n");
            for (element, glyphs) in &self.topbar_composition.empty_missing_glyphs {
                out.push_str(&format!("  {element}: {glyphs}\n"));
            }
        }
        if !self.topbar_composition.empty_unresolved_loc_keys.is_empty() {
            out.push_str("topbar composition empty unresolved loc keys:\n");
            for key in &self.topbar_composition.empty_unresolved_loc_keys {
                out.push_str(&format!("  {key}\n"));
            }
        }
        out.push_str("hud source windows:\n");
        for source in &self.hud_window.source_windows {
            out.push_str(&format!("  {source}\n"));
        }
        out.push_str(&format!(
            "hud sprite refs={} decoded={} missing={} undecodable={}\n",
            self.hud_window.referenced_sprites,
            self.hud_window.decoded_sprites,
            self.hud_window.missing_sprites.len(),
            self.hud_window.undecodable_sprites.len()
        ));
        if !self.hud_window.missing_sprites.is_empty() {
            out.push_str("hud missing sprites:\n");
            for item in &self.hud_window.missing_sprites {
                out.push_str(&format!("  {item}\n"));
            }
        }
        if !self.hud_window.undecodable_sprites.is_empty() {
            out.push_str("hud undecodable sprites:\n");
            for item in &self.hud_window.undecodable_sprites {
                out.push_str(&format!("  {item}\n"));
            }
        }
        out.push_str(&format!(
            "hud tooltip source note: {}\n",
            self.hud_window.tooltip_source_note
        ));
        out.push_str(&format!(
            "hud composition sample: drawn={} missing_sprites={} undecodable_sprites={} missing_glyph_sets={}\n",
            self.hud_composition.sample_drawn_elements,
            self.hud_composition.sample_missing_sprites,
            self.hud_composition.sample_undecodable_sprites,
            self.hud_composition.sample_missing_glyphs.len()
        ));
        if !self.hud_composition.sample_missing_glyphs.is_empty() {
            out.push_str("hud composition sample missing glyphs:\n");
            for (element, glyphs) in &self.hud_composition.sample_missing_glyphs {
                out.push_str(&format!("  {element}: {glyphs}\n"));
            }
        }
        if !self.hud_composition.sample_unresolved_loc_keys.is_empty() {
            out.push_str("hud composition sample unresolved loc keys:\n");
            for key in &self.hud_composition.sample_unresolved_loc_keys {
                out.push_str(&format!("  {key}\n"));
            }
        }
        out.push_str(&format!(
            "hud composition empty: drawn={} missing_sprites={} undecodable_sprites={} missing_glyph_sets={}\n",
            self.hud_composition.empty_drawn_elements,
            self.hud_composition.empty_missing_sprites,
            self.hud_composition.empty_undecodable_sprites,
            self.hud_composition.empty_missing_glyphs.len()
        ));
        if !self.hud_composition.empty_missing_glyphs.is_empty() {
            out.push_str("hud composition empty missing glyphs:\n");
            for (element, glyphs) in &self.hud_composition.empty_missing_glyphs {
                out.push_str(&format!("  {element}: {glyphs}\n"));
            }
        }
        if !self.hud_composition.empty_unresolved_loc_keys.is_empty() {
            out.push_str("hud composition empty unresolved loc keys:\n");
            for key in &self.hud_composition.empty_unresolved_loc_keys {
                out.push_str(&format!("  {key}\n"));
            }
        }
        out.push_str("major view compositions:\n");
        for view in &self.major_views {
            out.push_str(&format!(
                "  {}: drawn={} documented_skips={} missing_sprites={} undecodable_sprites={} missing_fonts={} missing_glyph_sets={} unresolved_loc_keys={} png={}\n",
                view.display_name,
                view.sample_drawn_elements,
                view.documented_skips.len(),
                view.sample_missing_sprites,
                view.sample_undecodable_sprites,
                view.sample_missing_fonts,
                view.sample_missing_glyphs.len(),
                view.sample_unresolved_loc_keys.len(),
                view.proof_png
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "<not written>".to_string())
            ));
            out.push_str("    sources:\n");
            for source in &view.source_files {
                out.push_str(&format!("      {source}\n"));
            }
            out.push_str("    roots/templates:\n");
            for root in &view.root_windows {
                out.push_str(&format!("      {root}\n"));
            }
            if !view.sample_missing_sprite_names.is_empty() {
                out.push_str("    missing sprites:\n");
                for item in &view.sample_missing_sprite_names {
                    out.push_str(&format!("      {item}\n"));
                }
            }
            if !view.sample_undecodable_sprite_errors.is_empty() {
                out.push_str("    undecodable sprites:\n");
                for item in &view.sample_undecodable_sprite_errors {
                    out.push_str(&format!("      {item}\n"));
                }
            }
            if !view.sample_missing_font_errors.is_empty() {
                out.push_str("    missing fonts:\n");
                for item in &view.sample_missing_font_errors {
                    out.push_str(&format!("      {item}\n"));
                }
            }
            if !view.sample_missing_glyphs.is_empty() {
                out.push_str("    missing glyphs:\n");
                for (element, glyphs) in &view.sample_missing_glyphs {
                    out.push_str(&format!("      {element}: {glyphs}\n"));
                }
            }
            if !view.sample_unresolved_loc_keys.is_empty() {
                out.push_str("    unresolved loc keys:\n");
                for key in &view.sample_unresolved_loc_keys {
                    out.push_str(&format!("      {key}\n"));
                }
            }
            if !view.documented_skips.is_empty() {
                out.push_str("    documented skips:\n");
                for item in &view.documented_skips {
                    out.push_str(&format!("      {item}\n"));
                }
            }
        }
        out.push_str(&format!(
            "fonts checked={} missing_ascii_sets={} missing_topbar_sample_sets={}\n",
            self.fonts.checked_fonts.join(","),
            self.fonts.missing_ascii.len(),
            self.fonts.missing_topbar_sample_glyphs.len()
        ));
        if !self.fonts.missing_ascii.is_empty() {
            out.push_str("font missing ASCII:\n");
            for (font, glyphs) in &self.fonts.missing_ascii {
                out.push_str(&format!("  {font}: {glyphs}\n"));
            }
        }
        if !self.fonts.missing_topbar_sample_glyphs.is_empty() {
            out.push_str("font missing topbar sample glyphs:\n");
            for (font, glyphs) in &self.fonts.missing_topbar_sample_glyphs {
                out.push_str(&format!("  {font}: {glyphs}\n"));
            }
        }
        out.push_str(&format!(
            "referenced DDS textures={} unreadable={}\n",
            self.referenced_dds.texture_count,
            self.referenced_dds.unreadable.len()
        ));
        out.push_str("referenced DDS format histogram:\n");
        push_histogram(&mut out, &self.referenced_dds.format_histogram);
        out.push_str("unsupported referenced DDS formats:\n");
        push_histogram(&mut out, &self.referenced_dds.unsupported_histogram);
        if !self.referenced_dds.unreadable.is_empty() {
            out.push_str("referenced DDS unreadable:\n");
            for item in self.referenced_dds.unreadable.iter().take(50) {
                out.push_str(&format!("  {item}\n"));
            }
            if self.referenced_dds.unreadable.len() > 50 {
                out.push_str(&format!(
                    "  ... {} more\n",
                    self.referenced_dds.unreadable.len() - 50
                ));
            }
        }
        out.push_str("png dumps:\n");
        for path in &self.png_dumps {
            out.push_str(&format!("  {}\n", path.display()));
        }
        out.push_str("parser design notes:\n");
        for note in &self.parser_notes {
            out.push_str(&format!("  {note}\n"));
        }
        out
    }
}

fn push_histogram(out: &mut String, histogram: &BTreeMap<String, usize>) {
    if histogram.is_empty() {
        out.push_str("  <none>\n");
        return;
    }
    for (key, count) in histogram {
        out.push_str(&format!("  {key}: {count}\n"));
    }
}
