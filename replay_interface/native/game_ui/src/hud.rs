use crate::assets::{normalize_rel_path, AssetResolver};
use crate::compose::{
    draw_image, draw_nine_slice, draw_rect_outline, draw_scaled, fill_rect,
    measure_to_pixels_for_parent, resolve_element_rect, ComposeOptions, ComposeOutput,
    ComposeReport, Compositor, DebugRect, Rect,
};
use crate::dds::RgbaImage;
use crate::font::{blank_image, BitmapFont, TextAlign};
use crate::gfx::{Size, SpriteRegistry};
use crate::gui::{parse_measure, GuiElement, LayoutSet, MeasurePair};
use crate::loc::Localisation;
use crate::paradox_text::{parse_file, scalar_i32, Value};
use crate::png::write_png;
use crate::topbar::{compose_topbar_at, topbar_debug_overlay, TopbarBinding};
use crate::{GameUiError, Result};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub const MINIMAP_GUI: &str = "interface/minimap.gui";
pub const MINIMAP_WINDOW_NAME: &str = "mini_map_interface";
pub const MAPMODES_GUI: &str = "interface/mapmodes_interface.gui";
pub const MAPMODES_WINDOW_NAME: &str = "MapmodesInterface_Ingame";
pub const MENUBAR_GUI: &str = "interface/menubar.gui";
pub const OUTLINER_GUI: &str = "interface/theatreselector.gui";
pub const OUTLINER_WINDOW_NAME: &str = "theatre_selection_window";
pub const OUTLINER_ROW_WINDOW_NAME: &str = "theatre_selection_entry";
pub const CORE_GUI: &str = "interface/core.gui";
pub const TOOLTIP_WINDOW_NAME: &str = "ToolTip";
pub const TOOLTIP_FONT_FALLBACK: &str = "hoi_16tooltip3";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HudSourceWindow {
    pub panel: &'static str,
    pub source_file: String,
    pub window_name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HudLayouts {
    pub minimap: GuiElement,
    pub mapmodes: GuiElement,
    pub zoom_in: GuiElement,
    pub zoom_out: GuiElement,
    pub outliner_shell: GuiElement,
    pub outliner_row: GuiElement,
    pub tooltip_shell: GuiElement,
    pub sources: Vec<HudSourceWindow>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HudBinding {
    pub topbar: TopbarBinding,
    pub mapmode: MapmodeBinding,
    pub minimap: MinimapBinding,
    pub outliner: OutlinerBinding,
    pub tooltip: Option<TooltipBinding>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapmodeBinding {
    pub selected_button: u8,
    pub use_four_button_layout: bool,
}

impl Default for MapmodeBinding {
    fn default() -> Self {
        Self {
            selected_button: 1,
            use_four_button_layout: true,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MinimapBinding {
    pub image: Option<RgbaImage>,
    pub view_rect: Option<MinimapViewRect>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MinimapViewRect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OutlinerBinding {
    pub collapsed: bool,
    pub rows: Vec<OutlinerRowBinding>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OutlinerRowBinding {
    pub icon_sprite: String,
    pub text: String,
    pub progress: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TooltipBinding {
    pub x: i32,
    pub y: i32,
    pub width: usize,
    pub height: usize,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HudSpriteStats {
    pub referenced: usize,
    pub decoded: usize,
    pub missing: Vec<String>,
    pub undecodable: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HudComposeResult {
    pub output: ComposeOutput,
    pub sources: Vec<HudSourceWindow>,
    pub sprite_stats: HudSpriteStats,
}

impl HudBinding {
    pub fn sample() -> Self {
        Self {
            topbar: TopbarBinding::sample_ire(),
            mapmode: MapmodeBinding {
                selected_button: 5,
                use_four_button_layout: true,
            },
            minimap: MinimapBinding {
                image: Some(sample_minimap_image(343, 98)),
                view_rect: Some(MinimapViewRect {
                    x: 360,
                    y: 320,
                    width: 180,
                    height: 140,
                }),
            },
            outliner: OutlinerBinding {
                collapsed: false,
                rows: vec![
                    OutlinerRowBinding {
                        icon_sprite: "GFX_theatre_army_shield".to_string(),
                        text: "1st Army - \u{00a7}GReady\u{00a7}!".to_string(),
                        progress: Some(0.84),
                    },
                    OutlinerRowBinding {
                        icon_sprite: "GFX_theatre_task_force_shield".to_string(),
                        text: "Atlantic Patrol".to_string(),
                        progress: Some(0.42),
                    },
                    OutlinerRowBinding {
                        icon_sprite: "GFX_theatre_air_force_shield".to_string(),
                        text: "Fighter Command".to_string(),
                        progress: Some(0.65),
                    },
                    OutlinerRowBinding {
                        icon_sprite: "GFX_production_output_icon".to_string(),
                        text: "Infantry Equipment".to_string(),
                        progress: Some(0.31),
                    },
                ],
            },
            tooltip: Some(TooltipBinding {
                x: 1120,
                y: 190,
                width: 360,
                height: 116,
                text: "Supply Area\nProjected supply remains \u{00a7}Gstable\u{00a7}! but rail throughput is \u{00a7}Ylimited\u{00a7}! while front-line depots recover from recent fighting."
                    .to_string(),
            }),
        }
    }
}

pub fn load_hud_layouts(resolver: &AssetResolver) -> Result<HudLayouts> {
    let minimap_file = resolver.resolve_file(MINIMAP_GUI)?;
    let mapmodes_file = resolver.resolve_file(MAPMODES_GUI)?;
    let menubar_file = resolver.resolve_file(MENUBAR_GUI)?;
    let outliner_file = resolver.resolve_file(OUTLINER_GUI)?;
    let core_file = resolver.resolve_file(CORE_GUI)?;

    let minimap = load_window_from_file(
        &minimap_file.path,
        &minimap_file.rel_path,
        MINIMAP_WINDOW_NAME,
    )?;
    let mapmodes = load_window_from_file(
        &mapmodes_file.path,
        &mapmodes_file.rel_path,
        MAPMODES_WINDOW_NAME,
    )?;
    let zoom_in =
        load_named_element_from_file(&menubar_file.path, &menubar_file.rel_path, "zoom_in")?;
    let zoom_out =
        load_named_element_from_file(&menubar_file.path, &menubar_file.rel_path, "zoom_out")?;
    let outliner_shell = load_window_from_file(
        &outliner_file.path,
        &outliner_file.rel_path,
        OUTLINER_WINDOW_NAME,
    )?;
    let outliner_row = load_window_from_file(
        &outliner_file.path,
        &outliner_file.rel_path,
        OUTLINER_ROW_WINDOW_NAME,
    )?;
    let tooltip_shell =
        load_named_element_from_file(&core_file.path, &core_file.rel_path, TOOLTIP_WINDOW_NAME)?;

    Ok(HudLayouts {
        minimap,
        mapmodes,
        zoom_in,
        zoom_out,
        outliner_shell,
        outliner_row,
        tooltip_shell,
        sources: vec![
            HudSourceWindow {
                panel: "minimap",
                source_file: minimap_file.rel_path,
                window_name: MINIMAP_WINDOW_NAME.to_string(),
            },
            HudSourceWindow {
                panel: "map_modes",
                source_file: mapmodes_file.rel_path,
                window_name: MAPMODES_WINDOW_NAME.to_string(),
            },
            HudSourceWindow {
                panel: "zoom_controls",
                source_file: menubar_file.rel_path,
                window_name: "zoom_in / zoom_out".to_string(),
            },
            HudSourceWindow {
                panel: "outliner_shell",
                source_file: outliner_file.rel_path.clone(),
                window_name: OUTLINER_WINDOW_NAME.to_string(),
            },
            HudSourceWindow {
                panel: "outliner_row",
                source_file: outliner_file.rel_path,
                window_name: OUTLINER_ROW_WINDOW_NAME.to_string(),
            },
            HudSourceWindow {
                panel: "tooltip_shell",
                source_file: core_file.rel_path,
                window_name: TOOLTIP_WINDOW_NAME.to_string(),
            },
        ],
    })
}

pub fn compose_hud(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    binding: &HudBinding,
    width: usize,
    height: usize,
) -> Result<HudComposeResult> {
    let layouts = load_hud_layouts(resolver)?;
    let mut output = empty_output(width, height);
    let mut stats = HudSpriteStats::default();

    let topbar = compose_topbar_at(resolver, registry, &binding.topbar, width, height)?;
    draw_image(&mut output.image, &topbar.output.image, 0, 0);
    merge_output(&mut output, topbar.output);

    compose_minimap_into(
        resolver,
        registry,
        &layouts,
        &binding.mapmode,
        &binding.minimap,
        &mut output,
        &mut stats,
    )?;
    compose_outliner_into(
        resolver,
        registry,
        &layouts,
        &binding.outliner,
        &mut output,
        &mut stats,
    )?;
    if let Some(tooltip) = &binding.tooltip {
        compose_tooltip_into(
            resolver,
            registry,
            &layouts,
            tooltip,
            &mut output,
            &mut stats,
        )?;
    }

    Ok(HudComposeResult {
        output,
        sources: layouts.sources,
        sprite_stats: stats,
    })
}

pub fn compose_minimap_panel(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    binding: &MinimapBinding,
    width: usize,
    height: usize,
) -> Result<HudComposeResult> {
    let layouts = load_hud_layouts(resolver)?;
    let mut output = empty_output(width, height);
    let mut stats = HudSpriteStats::default();
    let mapmode = MapmodeBinding::default();
    compose_minimap_into(
        resolver,
        registry,
        &layouts,
        &mapmode,
        binding,
        &mut output,
        &mut stats,
    )?;
    Ok(HudComposeResult {
        output,
        sources: layouts.sources,
        sprite_stats: stats,
    })
}

pub fn compose_mapmode_panel(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    binding: &MapmodeBinding,
    width: usize,
    height: usize,
) -> Result<HudComposeResult> {
    let layouts = load_hud_layouts(resolver)?;
    let mut output = empty_output(width, height);
    let mut stats = HudSpriteStats::default();
    compose_mapmodes_into(
        resolver,
        registry,
        &layouts,
        binding,
        &mut output,
        &mut stats,
    )?;
    Ok(HudComposeResult {
        output,
        sources: layouts.sources,
        sprite_stats: stats,
    })
}

pub fn compose_outliner_panel(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    binding: &OutlinerBinding,
    width: usize,
    height: usize,
) -> Result<HudComposeResult> {
    let layouts = load_hud_layouts(resolver)?;
    let mut output = empty_output(width, height);
    let mut stats = HudSpriteStats::default();
    compose_outliner_into(
        resolver,
        registry,
        &layouts,
        binding,
        &mut output,
        &mut stats,
    )?;
    Ok(HudComposeResult {
        output,
        sources: layouts.sources,
        sprite_stats: stats,
    })
}

pub fn compose_tooltip_panel(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    binding: &TooltipBinding,
    width: usize,
    height: usize,
) -> Result<HudComposeResult> {
    let layouts = load_hud_layouts(resolver)?;
    let mut output = empty_output(width, height);
    let mut stats = HudSpriteStats::default();
    compose_tooltip_into(
        resolver,
        registry,
        &layouts,
        binding,
        &mut output,
        &mut stats,
    )?;
    Ok(HudComposeResult {
        output,
        sources: layouts.sources,
        sprite_stats: stats,
    })
}

pub fn write_hud_proofs(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    out_dir: &Path,
) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    let binding = HudBinding::sample();

    let hud = compose_hud(resolver, registry, &binding, 1920, 1080)?;
    let hud_path = out_dir.join("hud_composited_1920x1080.png");
    write_png(&hud_path, &hud.output.image)?;
    paths.push(hud_path);

    let debug = topbar_debug_overlay(resolver, &hud.output)?;
    let debug_path = out_dir.join("hud_debug_rects_1920x1080.png");
    write_png(&debug_path, &debug)?;
    paths.push(debug_path);

    let hud_1440 = compose_hud(resolver, registry, &binding, 2560, 1440)?;
    let hud_1440_path = out_dir.join("hud_composited_2560x1440.png");
    write_png(&hud_1440_path, &hud_1440.output.image)?;
    paths.push(hud_1440_path);

    let debug_1440 = topbar_debug_overlay(resolver, &hud_1440.output)?;
    let debug_1440_path = out_dir.join("hud_debug_rects_2560x1440.png");
    write_png(&debug_1440_path, &debug_1440)?;
    paths.push(debug_1440_path);

    let mapmodes = compose_mapmode_panel(resolver, registry, &binding.mapmode, 1920, 1080)?;
    let mapmodes_path = out_dir.join("mapmode_row_selected_button_5.png");
    write_png(
        &mapmodes_path,
        &crop_nontransparent(&mapmodes.output.image, 12),
    )?;
    paths.push(mapmodes_path);

    let minimap = compose_minimap_panel(resolver, registry, &binding.minimap, 1920, 1080)?;
    let minimap_path = out_dir.join("minimap_panel.png");
    write_png(
        &minimap_path,
        &crop_nontransparent(&minimap.output.image, 12),
    )?;
    paths.push(minimap_path);

    let outliner = compose_outliner_panel(resolver, registry, &binding.outliner, 1920, 1080)?;
    let outliner_path = out_dir.join("outliner_sample.png");
    write_png(
        &outliner_path,
        &crop_nontransparent(&outliner.output.image, 12),
    )?;
    paths.push(outliner_path);

    let mut tooltip_binding = binding.tooltip.clone().expect("sample has tooltip");
    tooltip_binding.x = 12;
    tooltip_binding.y = 12;
    tooltip_binding.width = 300;
    tooltip_binding.height = 138;
    let tooltip = compose_tooltip_panel(resolver, registry, &tooltip_binding, 640, 220)?;
    let tooltip_path = out_dir.join("tooltip_wrapped_multiline.png");
    write_png(
        &tooltip_path,
        &crop_nontransparent(&tooltip.output.image, 12),
    )?;
    paths.push(tooltip_path);

    Ok(paths)
}

pub fn hud_debug_overlay(resolver: &AssetResolver, output: &ComposeOutput) -> Result<RgbaImage> {
    topbar_debug_overlay(resolver, output)
}

pub fn inspect_hud_sprites(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
) -> Result<HudSpriteStats> {
    let layouts = load_hud_layouts(resolver)?;
    let mut names = BTreeSet::new();
    for root in [
        &layouts.minimap,
        &layouts.mapmodes,
        &layouts.zoom_in,
        &layouts.zoom_out,
        &layouts.outliner_shell,
        &layouts.outliner_row,
        &layouts.tooltip_shell,
    ] {
        root.collect_sprite_refs(&mut names);
    }
    for name in [
        "GFX_tiled_window_transparent",
        "GFX_theatre_selection_background",
        "GFX_theatre_selection_frame",
        "GFX_theatre_army_shield",
        "GFX_theatre_task_force_shield",
        "GFX_theatre_air_force_shield",
        "GFX_topbar_fuel_bar",
        "GFX_production_output_icon",
    ] {
        names.insert(name.to_string());
    }
    Ok(sprite_stats(resolver, registry, names))
}

fn compose_minimap_into(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    layouts: &HudLayouts,
    mapmode: &MapmodeBinding,
    binding: &MinimapBinding,
    output: &mut ComposeOutput,
    stats: &mut HudSpriteStats,
) -> Result<()> {
    let mut options = localized_options(resolver, output.image.width, output.image.height)?;
    if let Some(minimap) = &binding.image {
        let (w, h) = sprite_first_frame_size(resolver, registry, "GFX_minimap_world")?;
        options
            .sprite_overrides
            .insert("worldmap_bg".to_string(), scaled_image(minimap, w, h));
    }
    let minimap = compose_root(resolver, registry, &layouts.minimap, &options);
    let viewport = minimap
        .debug_rects
        .iter()
        .find(|rect| rect.name == "worldmap_bg")
        .cloned();
    draw_image(&mut output.image, &minimap.image, 0, 0);
    merge_output(output, minimap);

    compose_mapmodes_into(resolver, registry, layouts, mapmode, output, stats)?;

    for zoom in [&layouts.zoom_in, &layouts.zoom_out] {
        let zoom_output = compose_root(
            resolver,
            registry,
            zoom,
            &localized_options(resolver, output.image.width, output.image.height)?,
        );
        draw_image(&mut output.image, &zoom_output.image, 0, 0);
        merge_output(output, zoom_output);
    }

    if let (Some(viewport), Some(view_rect)) = (viewport, binding.view_rect) {
        let rect = minimap_view_rect_to_pixels(&viewport, view_rect);
        draw_rect_outline(&mut output.image, rect, [245, 213, 101, 230]);
        draw_rect_outline(
            &mut output.image,
            Rect::new(rect.x() + 1, rect.y() + 1, rect.w() - 2, rect.h() - 2),
            [20, 20, 18, 190],
        );
        output.debug_rects.push(DebugRect {
            name: "minimap_view_rect".to_string(),
            element_type: "binding".to_string(),
            x: rect.x(),
            y: rect.y(),
            w: rect.w(),
            h: rect.h(),
            depth: 2,
        });
    }

    *stats = merge_sprite_stats(
        stats,
        &sprite_stats_for_roots(
            resolver,
            registry,
            [&layouts.minimap, &layouts.zoom_in, &layouts.zoom_out],
        ),
    );
    Ok(())
}

fn compose_mapmodes_into(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    layouts: &HudLayouts,
    binding: &MapmodeBinding,
    output: &mut ComposeOutput,
    stats: &mut HudSpriteStats,
) -> Result<()> {
    let mut options = localized_options(resolver, output.image.width, output.image.height)?;
    apply_mapmode_binding(&layouts.mapmodes, registry, binding, &mut options);
    let mapmodes = compose_root(resolver, registry, &layouts.mapmodes, &options);
    draw_image(&mut output.image, &mapmodes.image, 0, 0);
    merge_output(output, mapmodes);
    *stats = merge_sprite_stats(
        stats,
        &sprite_stats_for_roots(resolver, registry, [&layouts.mapmodes]),
    );
    Ok(())
}

fn apply_mapmode_binding(
    root: &GuiElement,
    registry: &SpriteRegistry,
    binding: &MapmodeBinding,
    options: &mut ComposeOptions,
) {
    if binding.use_four_button_layout {
        options
            .hidden_elements
            .insert("3_button_mapmode".to_string());
    } else {
        options
            .hidden_elements
            .insert("4_button_mapmode".to_string());
    }

    let selected = (1..=12)
        .contains(&binding.selected_button)
        .then_some(binding.selected_button);
    for button in 1..=12u8 {
        if Some(button) == selected {
            options
                .hidden_elements
                .insert(format!("mapmode_button_{button}"));
        } else {
            options
                .hidden_elements
                .insert(format!("mapmode_button_{button}_s"));
            options
                .hidden_elements
                .insert(format!("mapmode_button_{button}_s_bg"));
        }
    }
    apply_mapmode_frame_overrides(root, registry, options);
}

fn apply_mapmode_frame_overrides(
    element: &GuiElement,
    registry: &SpriteRegistry,
    options: &mut ComposeOptions,
) {
    if let Some(button) = mapmode_button_number(&element.name) {
        if let Some(sprite_name) = element.sprite_refs.first() {
            if let Some(sprite) = registry.get(sprite_name) {
                let frame = mapmode_frame_for_button(button, sprite.no_of_frames as usize);
                options.frame_overrides.insert(element.name.clone(), frame);
            }
        }
    }
    for child in &element.children {
        apply_mapmode_frame_overrides(child, registry, options);
    }
}

pub fn mapmode_frame_for_button(button_number: u8, frame_count: usize) -> usize {
    if frame_count <= 1 {
        return 0;
    }
    usize::from(button_number.saturating_sub(1)).min(frame_count - 1)
}

fn mapmode_button_number(name: &str) -> Option<u8> {
    let tail = name.strip_prefix("mapmode_button_")?;
    let tail = tail.strip_suffix("_s").unwrap_or(tail);
    if tail.contains('_') {
        return None;
    }
    tail.parse::<u8>().ok()
}

fn compose_outliner_into(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    layouts: &HudLayouts,
    binding: &OutlinerBinding,
    output: &mut ComposeOutput,
    stats: &mut HudSpriteStats,
) -> Result<()> {
    let shell = compose_root(
        resolver,
        registry,
        &layouts.outliner_shell,
        &localized_options(resolver, output.image.width, output.image.height)?,
    );
    draw_image(&mut output.image, &shell.image, 0, 0);
    merge_output(output, shell);

    let panel = outliner_panel_rect(
        &layouts.outliner_shell,
        output.image.width,
        output.image.height,
    );
    draw_sprite_scaled(
        resolver,
        registry,
        &mut output.image,
        "GFX_tiled_window_transparent",
        panel,
        &mut output.report,
    );
    output.debug_rects.push(DebugRect {
        name: "outliner_bound_rows".to_string(),
        element_type: "binding".to_string(),
        x: panel.x(),
        y: panel.y(),
        w: panel.w(),
        h: panel.h(),
        depth: 1,
    });

    let header = Rect::new(panel.x() + 6, panel.y() + 6, panel.w() - 12, 32);
    fill_rect(&mut output.image, header, [23, 27, 30, 185]);
    for (idx, sprite) in [
        "GFX_theatre_army_shield",
        "GFX_theatre_task_force_shield",
        "GFX_theatre_air_force_shield",
    ]
    .iter()
    .enumerate()
    {
        draw_sprite_scaled(
            resolver,
            registry,
            &mut output.image,
            sprite,
            Rect::new(header.x() + 8 + idx as i32 * 34, header.y() + 4, 25, 25),
            &mut output.report,
        );
    }

    if !binding.collapsed {
        draw_outliner_rows(
            resolver,
            registry,
            &mut output.image,
            &binding.rows,
            Rect::new(
                panel.x() + 8,
                panel.y() + 44,
                panel.w() - 16,
                panel.h() - 52,
            ),
            &mut output.report,
            &mut output.debug_rects,
        )?;
    }

    let manual = inspect_hud_sprites(resolver, registry)?;
    *stats = merge_sprite_stats(stats, &manual);
    Ok(())
}

fn compose_tooltip_into(
    resolver: &AssetResolver,
    _registry: &SpriteRegistry,
    layouts: &HudLayouts,
    binding: &TooltipBinding,
    output: &mut ComposeOutput,
    _stats: &mut HudSpriteStats,
) -> Result<()> {
    let rect = Rect::new(
        binding.x,
        binding.y,
        binding.width as i32,
        binding.height as i32,
    );
    let tooltip = &layouts.tooltip_shell;
    let texture_file = tooltip
        .raw
        .first_scalar_ci("textureFile")
        .map(normalize_rel_path)
        .ok_or_else(|| GameUiError::missing("ToolTip textureFile missing"))?;
    let texture = resolver.decode_texture(&texture_file)?.image;
    let (border_x, border_y) = border_size(&tooltip.raw).unwrap_or((18, 18));
    draw_nine_slice(
        &mut output.image,
        &texture,
        rect.x(),
        rect.y(),
        rect.w().max(0) as usize,
        rect.h().max(0) as usize,
        border_x,
        border_y,
    );
    let font_name = tooltip.font.as_deref().unwrap_or(TOOLTIP_FONT_FALLBACK);
    let mut font = BitmapFont::load(resolver, font_name)
        .or_else(|_| BitmapFont::load(resolver, TOOLTIP_FONT_FALLBACK))?;
    if font.name.eq_ignore_ascii_case("cg_16b") {
        font.glyph_channel = None;
    }
    let text_rect = Rect::new(rect.x() + 12, rect.y() + 10, rect.w() - 24, rect.h() - 16);
    let wrapped = wrap_tooltip_text_with(&binding.text, text_rect.w().max(8), |text| {
        font.measure_text(text)
    });
    let line_step = font.line_height.max(18);
    for (line_idx, line) in wrapped.lines().enumerate() {
        let line_y = text_rect.y() + line_idx as i32 * line_step;
        if line_y >= text_rect.y() + text_rect.h() {
            break;
        }
        let text_report = font.draw_text(
            &mut output.image,
            text_rect.x(),
            line_y,
            Some(text_rect.w()),
            TextAlign::Left,
            line,
            tooltip_line_color(line_idx, line),
        );
        if !text_report.missing_glyphs.is_empty() {
            output
                .report
                .missing_glyphs
                .entry("tooltip_text".to_string())
                .or_default()
                .extend(text_report.missing_glyphs);
        }
    }
    output.report.drawn_elements += 1;
    output.debug_rects.push(DebugRect {
        name: "tooltip_shell".to_string(),
        element_type: TOOLTIP_WINDOW_NAME.to_string(),
        x: rect.x(),
        y: rect.y(),
        w: rect.w(),
        h: rect.h(),
        depth: 1,
    });
    output.debug_rects.push(DebugRect {
        name: "tooltip_text".to_string(),
        element_type: "instantTextBoxType".to_string(),
        x: text_rect.x(),
        y: text_rect.y(),
        w: text_rect.w(),
        h: text_rect.h(),
        depth: 2,
    });
    Ok(())
}

fn draw_outliner_rows(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    image: &mut RgbaImage,
    rows: &[OutlinerRowBinding],
    area: Rect,
    report: &mut ComposeReport,
    debug_rects: &mut Vec<DebugRect>,
) -> Result<()> {
    let font = BitmapFont::load(resolver, "hoi_18b")
        .or_else(|_| BitmapFont::load(resolver, "hoi_18mbs"))?;
    let row_h = 58;
    for (idx, row) in rows.iter().enumerate() {
        let y = area.y() + idx as i32 * row_h;
        if y + row_h > area.y() + area.h() {
            break;
        }
        let row_rect = Rect::new(area.x(), y, area.w(), row_h - 4);
        draw_sprite_scaled(
            resolver,
            registry,
            image,
            "GFX_theatre_selection_background",
            row_rect,
            report,
        );
        draw_sprite_scaled(
            resolver,
            registry,
            image,
            "GFX_theatre_selection_frame",
            row_rect,
            report,
        );
        draw_sprite_scaled(
            resolver,
            registry,
            image,
            &row.icon_sprite,
            Rect::new(row_rect.x() + 10, row_rect.y() + 10, 30, 30),
            report,
        );
        let text_report = font.draw_text(
            image,
            row_rect.x() + 50,
            row_rect.y() + 8,
            Some(row_rect.w() - 68),
            TextAlign::Left,
            &row.text,
            [221, 214, 190, 255],
        );
        if !text_report.missing_glyphs.is_empty() {
            report
                .missing_glyphs
                .entry(format!("outliner_row_{idx}"))
                .or_default()
                .extend(text_report.missing_glyphs);
        }
        report.drawn_elements += 1;
        if let Some(progress) = row.progress {
            draw_progress_bar(
                resolver,
                registry,
                image,
                Rect::new(row_rect.x() + 50, row_rect.y() + 34, row_rect.w() - 74, 7),
                progress,
                report,
            );
        }
        debug_rects.push(DebugRect {
            name: format!("outliner_row_{idx}"),
            element_type: OUTLINER_ROW_WINDOW_NAME.to_string(),
            x: row_rect.x(),
            y: row_rect.y(),
            w: row_rect.w(),
            h: row_rect.h(),
            depth: 2,
        });
    }
    Ok(())
}

fn draw_progress_bar(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    image: &mut RgbaImage,
    rect: Rect,
    progress: f32,
    report: &mut ComposeReport,
) {
    let Some(sprite) = registry.get("GFX_topbar_fuel_bar") else {
        report
            .missing_sprites
            .insert("GFX_topbar_fuel_bar".to_string());
        fill_rect(image, rect, [76, 116, 70, 220]);
        return;
    };
    let Some(fg_rel) = sprite.texture_files.first().map(|slot| slot.path.as_str()) else {
        report
            .undecodable_sprites
            .push("GFX_topbar_fuel_bar: no foreground texture".to_string());
        fill_rect(image, rect, [76, 116, 70, 220]);
        return;
    };
    let bg_rel = sprite
        .texture_files
        .get(1)
        .map(|slot| slot.path.as_str())
        .unwrap_or(fg_rel);
    match (
        resolver.decode_texture(fg_rel),
        resolver.decode_texture(bg_rel),
    ) {
        (Ok(fg), Ok(bg)) => {
            draw_scaled(
                image,
                &bg.image,
                rect.x(),
                rect.y(),
                rect.w().max(1) as usize,
                rect.h().max(1) as usize,
            );
            let fill_w = ((rect.w().max(0) as f32) * progress.clamp(0.0, 1.0)).round() as i32;
            if fill_w > 0 {
                draw_scaled(
                    image,
                    &fg.image,
                    rect.x(),
                    rect.y(),
                    fill_w as usize,
                    rect.h().max(1) as usize,
                );
            }
            report.drawn_elements += 1;
        }
        (fg, bg) => {
            if let Err(err) = fg {
                report
                    .undecodable_sprites
                    .push(format!("GFX_topbar_fuel_bar: {}", err.message));
            }
            if let Err(err) = bg {
                report
                    .undecodable_sprites
                    .push(format!("GFX_topbar_fuel_bar background: {}", err.message));
            }
            fill_rect(image, rect, [76, 116, 70, 220]);
        }
    }
}

fn compose_root(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    root: &GuiElement,
    options: &ComposeOptions,
) -> ComposeOutput {
    let mut compositor = Compositor::new(resolver, registry);
    compositor.compose(root, options)
}

fn empty_output(width: usize, height: usize) -> ComposeOutput {
    ComposeOutput {
        image: blank_image(width, height),
        report: ComposeReport::default(),
        debug_rects: Vec::new(),
    }
}

fn localized_options(
    resolver: &AssetResolver,
    width: usize,
    height: usize,
) -> Result<ComposeOptions> {
    let localisation = Localisation::load_english(resolver)?;
    Ok(ComposeOptions::new(width, height).with_localisation(&localisation))
}

fn merge_output(dst: &mut ComposeOutput, src: ComposeOutput) {
    merge_report(&mut dst.report, src.report);
    dst.debug_rects.extend(src.debug_rects);
}

fn merge_report(dst: &mut ComposeReport, src: ComposeReport) {
    dst.drawn_elements += src.drawn_elements;
    dst.missing_sprites.extend(src.missing_sprites);
    dst.undecodable_sprites.extend(src.undecodable_sprites);
    dst.missing_fonts.extend(src.missing_fonts);
    for (element, glyphs) in src.missing_glyphs {
        dst.missing_glyphs
            .entry(element)
            .or_default()
            .extend(glyphs);
    }
    dst.unresolved_loc_keys.extend(src.unresolved_loc_keys);
}

fn load_window_from_file(path: &Path, rel: &str, name: &str) -> Result<GuiElement> {
    let doc = parse_file(path)?;
    let mut layouts = LayoutSet::default();
    layouts.add_document(rel, &doc);
    layouts
        .find_by_name(name)
        .into_iter()
        .find(|element| {
            element
                .element_type
                .eq_ignore_ascii_case("containerWindowType")
                || element.element_type.eq_ignore_ascii_case("windowType")
        })
        .cloned()
        .ok_or_else(|| GameUiError::missing(format!("{rel}: window {name} not found")))
}

fn load_named_element_from_file(path: &Path, rel: &str, name: &str) -> Result<GuiElement> {
    let doc = parse_file(path)?;
    let mut layouts = LayoutSet::default();
    layouts.add_document(rel, &doc);
    layouts
        .find_by_name(name)
        .into_iter()
        .next()
        .cloned()
        .ok_or_else(|| GameUiError::missing(format!("{rel}: element {name} not found")))
}

fn sprite_first_frame_size(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    sprite_name: &str,
) -> Result<(usize, usize)> {
    let sprite = registry
        .get(sprite_name)
        .ok_or_else(|| GameUiError::missing(format!("sprite {sprite_name} not found")))?;
    let decoded = resolver.decode_sprite(sprite)?;
    decoded
        .frames
        .first()
        .map(|frame| (frame.width, frame.height))
        .ok_or_else(|| GameUiError::missing(format!("sprite {sprite_name} has no frames")))
}

fn scaled_image(src: &RgbaImage, width: usize, height: usize) -> RgbaImage {
    let mut out = blank_image(width, height);
    draw_scaled(&mut out, src, 0, 0, width, height);
    out
}

fn draw_sprite_scaled(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    image: &mut RgbaImage,
    sprite_name: &str,
    rect: Rect,
    report: &mut ComposeReport,
) {
    if rect.w() <= 0 || rect.h() <= 0 {
        return;
    }
    let Some(sprite) = registry.get(sprite_name) else {
        report.missing_sprites.insert(sprite_name.to_string());
        return;
    };
    let decoded = match resolver.decode_sprite(sprite) {
        Ok(decoded) => decoded,
        Err(err) => {
            report
                .undecodable_sprites
                .push(format!("{sprite_name}: {}", err.message));
            return;
        }
    };
    let Some(frame) = decoded.frames.first() else {
        report
            .undecodable_sprites
            .push(format!("{sprite_name}: no decoded frames"));
        return;
    };
    if let Some(border) = &sprite.border_size {
        let (bx, by) = parse_sprite_size(border).unwrap_or((0, 0));
        draw_nine_slice(
            image,
            frame,
            rect.x(),
            rect.y(),
            rect.w().max(1) as usize,
            rect.h().max(1) as usize,
            bx,
            by,
        );
    } else {
        draw_scaled(
            image,
            frame,
            rect.x(),
            rect.y(),
            rect.w().max(1) as usize,
            rect.h().max(1) as usize,
        );
    }
    report.drawn_elements += 1;
}

fn parse_sprite_size(size: &Size) -> Option<(usize, usize)> {
    let x = size.x.parse::<f32>().ok()?.round().max(0.0) as usize;
    let y = size.y.parse::<f32>().ok()?.round().max(0.0) as usize;
    Some((x, y))
}

fn minimap_view_rect_to_pixels(viewport: &DebugRect, rect: MinimapViewRect) -> Rect {
    let vx = viewport.x;
    let vy = viewport.y;
    let vw = viewport.w.max(1);
    let vh = viewport.h.max(1);
    let x = vx + ((vw as i64 * i64::from(rect.x.min(1000))) / 1000) as i32;
    let y = vy + ((vh as i64 * i64::from(rect.y.min(1000))) / 1000) as i32;
    let w = ((vw as i64 * i64::from(rect.width.min(1000))) / 1000)
        .max(1)
        .min(i64::from(vx + vw - x)) as i32;
    let h = ((vh as i64 * i64::from(rect.height.min(1000))) / 1000)
        .max(1)
        .min(i64::from(vy + vh - y)) as i32;
    Rect::new(x, y, w, h)
}

fn outliner_panel_rect(shell: &GuiElement, screen_w: usize, screen_h: usize) -> Rect {
    let options = ComposeOptions::new(screen_w, screen_h);
    let root = resolve_element_rect(
        shell,
        Rect::new(0, 0, screen_w as i32, screen_h as i32),
        (760, 0),
        &options,
    );
    let bg_offset = shell
        .children
        .iter()
        .find(|child| child.name.eq_ignore_ascii_case("Background"))
        .and_then(|child| child.position.as_ref())
        .map(|pos| {
            (
                measure_to_pixels_for_parent(&pos.x, root.w()),
                measure_to_pixels_for_parent(&pos.y, root.h()),
            )
        })
        .unwrap_or((400, 0));
    let x = root.x() + bg_offset.0;
    let y = root.y() + bg_offset.1;
    let h = (screen_h as i32 - y - 240).max(180);
    Rect::new(x, y, (screen_w as i32 - x).max(300), h)
}

fn sprite_stats_for_roots<'a>(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    roots: impl IntoIterator<Item = &'a GuiElement>,
) -> HudSpriteStats {
    let mut names = BTreeSet::new();
    for root in roots {
        root.collect_sprite_refs(&mut names);
    }
    sprite_stats(resolver, registry, names)
}

fn sprite_stats(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    names: impl IntoIterator<Item = String>,
) -> HudSpriteStats {
    let mut stats = HudSpriteStats::default();
    let mut seen = BTreeSet::new();
    for name in names {
        if !seen.insert(name.clone()) {
            continue;
        }
        stats.referenced += 1;
        let Some(sprite) = registry.get(&name) else {
            stats
                .missing
                .push(format!("{name}: sprite registry entry missing"));
            continue;
        };
        match resolver.decode_sprite(sprite) {
            Ok(_) => stats.decoded += 1,
            Err(err) => stats.undecodable.push(format!("{name}: {}", err.message)),
        }
    }
    stats.missing.sort();
    stats.undecodable.sort();
    stats
}

fn merge_sprite_stats(lhs: &HudSpriteStats, rhs: &HudSpriteStats) -> HudSpriteStats {
    let mut merged = lhs.clone();
    merged.referenced += rhs.referenced;
    merged.decoded += rhs.decoded;
    merged.missing.extend(rhs.missing.clone());
    merged.undecodable.extend(rhs.undecodable.clone());
    merged.missing.sort();
    merged.missing.dedup();
    merged.undecodable.sort();
    merged.undecodable.dedup();
    merged
}

fn border_size(value: &Value) -> Option<(usize, usize)> {
    let border = value.first_value_ci("borderSize")?;
    let x = border
        .first_value_ci("x")
        .and_then(scalar_i32)
        .unwrap_or(0)
        .max(0) as usize;
    let y = border
        .first_value_ci("y")
        .and_then(scalar_i32)
        .unwrap_or(0)
        .max(0) as usize;
    Some((x, y))
}

fn wrap_tooltip_text_with<F>(text: &str, max_width: i32, measure: F) -> String
where
    F: Fn(&str) -> i32,
{
    if max_width <= 0 {
        return text.to_string();
    }
    let mut out = Vec::new();
    for source_line in text.lines() {
        let mut current = String::new();
        for word in source_line.split_whitespace() {
            let candidate = if current.is_empty() {
                word.to_string()
            } else {
                format!("{current} {word}")
            };
            if !current.is_empty() && measure(&candidate) > max_width {
                out.push(current);
                current = word.to_string();
            } else {
                current = candidate;
            }
        }
        out.push(current);
    }
    out.join("\n")
}

fn tooltip_line_color(line_idx: usize, line: &str) -> [u8; 4] {
    if line_idx == 0 && !line.trim().is_empty() {
        [245, 213, 101, 255]
    } else {
        [221, 214, 190, 255]
    }
}

fn sample_minimap_image(width: usize, height: usize) -> RgbaImage {
    let mut image = blank_image(width, height);
    for y in 0..height {
        for x in 0..width {
            let offset = (y * width + x) * 4;
            let sea = y > height / 2 || x < width / 5;
            let ridge = (x + y * 2) % 37 < 3;
            let rgba = if sea {
                [38, 75, 92, 255]
            } else if ridge {
                [104, 104, 82, 255]
            } else {
                [76, 98, 58, 255]
            };
            image.pixels[offset..offset + 4].copy_from_slice(&rgba);
        }
    }
    image
}

fn crop_nontransparent(image: &RgbaImage, margin: usize) -> RgbaImage {
    let mut min_x = image.width;
    let mut min_y = image.height;
    let mut max_x = 0usize;
    let mut max_y = 0usize;
    let mut found = false;
    for y in 0..image.height {
        for x in 0..image.width {
            let alpha = image.pixels[(y * image.width + x) * 4 + 3];
            if alpha == 0 {
                continue;
            }
            found = true;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    if !found {
        return image.clone();
    }
    min_x = min_x.saturating_sub(margin);
    min_y = min_y.saturating_sub(margin);
    max_x = (max_x + margin).min(image.width - 1);
    max_y = (max_y + margin).min(image.height - 1);
    let width = max_x - min_x + 1;
    let height = max_y - min_y + 1;
    let mut pixels = vec![0u8; width * height * 4];
    for y in 0..height {
        let src = ((min_y + y) * image.width + min_x) * 4;
        let dst = y * width * 4;
        pixels[dst..dst + width * 4].copy_from_slice(&image.pixels[src..src + width * 4]);
    }
    RgbaImage {
        width,
        height,
        pixels,
    }
}

#[allow(dead_code)]
fn synthetic_element(
    element_type: &str,
    name: &str,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    sprite: Option<&str>,
) -> GuiElement {
    GuiElement {
        element_type: element_type.to_string(),
        name: name.to_string(),
        position: Some(MeasurePair {
            x: parse_measure(&x.to_string()),
            y: parse_measure(&y.to_string()),
        }),
        size: Some(MeasurePair {
            x: parse_measure(&w.to_string()),
            y: parse_measure(&h.to_string()),
        }),
        orientation: Some("UPPER_LEFT".to_string()),
        sprite_refs: sprite.into_iter().map(str::to_string).collect(),
        font: None,
        text: None,
        format: None,
        children: Vec::new(),
        raw: Value::Object(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::{SpriteDefinition, TextureSlot};

    #[test]
    fn lower_right_view_rect_clamps_to_minimap_viewport() {
        let viewport = DebugRect {
            name: "worldmap_bg".to_string(),
            element_type: "iconType".to_string(),
            x: 1577,
            y: 982,
            w: 343,
            h: 98,
            depth: 1,
        };
        let rect = minimap_view_rect_to_pixels(
            &viewport,
            MinimapViewRect {
                x: 900,
                y: 850,
                width: 300,
                height: 300,
            },
        );
        assert!(rect.x() >= viewport.x);
        assert!(rect.y() >= viewport.y);
        assert!(rect.x() + rect.w() <= viewport.x + viewport.w);
        assert!(rect.y() + rect.h() <= viewport.y + viewport.h);
    }

    #[test]
    fn tooltip_wrap_preserves_existing_lines_and_color_escapes() {
        let wrapped = wrap_tooltip_text_with(
            "Alpha \u{00a7}YBravo\u{00a7}! Charlie\nDelta Echo",
            100,
            |text| crate::font::strip_color_codes(text).chars().count() as i32 * 10,
        );
        assert_eq!(
            wrapped,
            "Alpha\n\u{00a7}YBravo\u{00a7}!\nCharlie\nDelta Echo"
        );
    }

    #[test]
    fn tooltip_first_line_uses_header_color_and_body_uses_default_color() {
        assert_eq!(tooltip_line_color(0, "Supply Area"), [245, 213, 101, 255]);
        assert_eq!(
            tooltip_line_color(1, "Projected supply"),
            [221, 214, 190, 255]
        );
    }

    #[test]
    fn mapmode_binding_selects_one_twin_and_sets_sprite_frames() {
        let mut registry = SpriteRegistry::new();
        registry.insert(test_sprite("GFX_mapmode_buttons_deselected_small", 18));
        registry.insert(test_sprite("GFX_mapmode_buttons_selected_small", 18));
        let root = GuiElement {
            element_type: "containerWindowType".to_string(),
            name: MAPMODES_WINDOW_NAME.to_string(),
            position: None,
            size: None,
            orientation: None,
            sprite_refs: Vec::new(),
            font: None,
            text: None,
            format: None,
            children: vec![
                synthetic_element(
                    "buttonType",
                    "mapmode_button_5",
                    0,
                    0,
                    20,
                    20,
                    Some("GFX_mapmode_buttons_deselected_small"),
                ),
                synthetic_element(
                    "buttonType",
                    "mapmode_button_5_s",
                    0,
                    0,
                    20,
                    20,
                    Some("GFX_mapmode_buttons_selected_small"),
                ),
                synthetic_element(
                    "iconType",
                    "mapmode_button_5_s_bg",
                    0,
                    0,
                    20,
                    20,
                    Some("GFX_mapmode_button_depressed"),
                ),
                synthetic_element(
                    "buttonType",
                    "mapmode_button_6_s",
                    0,
                    0,
                    20,
                    20,
                    Some("GFX_mapmode_buttons_selected_small"),
                ),
            ],
            raw: Value::Object(Vec::new()),
        };
        let mut options = ComposeOptions::new(1920, 1080);
        apply_mapmode_binding(
            &root,
            &registry,
            &MapmodeBinding {
                selected_button: 5,
                use_four_button_layout: true,
            },
            &mut options,
        );

        assert!(options.hidden_elements.contains("3_button_mapmode"));
        assert!(options.hidden_elements.contains("mapmode_button_5"));
        assert!(!options.hidden_elements.contains("mapmode_button_5_s"));
        assert!(!options.hidden_elements.contains("mapmode_button_5_s_bg"));
        assert!(options.hidden_elements.contains("mapmode_button_6_s"));
        assert_eq!(options.frame_overrides.get("mapmode_button_5"), Some(&4));
        assert_eq!(options.frame_overrides.get("mapmode_button_5_s"), Some(&4));
        assert_eq!(mapmode_frame_for_button(12, 18), 11);
    }

    #[test]
    fn outliner_uses_right_side_orientation_math() {
        let shell = GuiElement {
            element_type: "containerWindowType".to_string(),
            name: OUTLINER_WINDOW_NAME.to_string(),
            position: Some(MeasurePair {
                x: parse_measure("-760"),
                y: parse_measure("117"),
            }),
            size: Some(MeasurePair {
                x: parse_measure("760"),
                y: parse_measure("-240"),
            }),
            orientation: Some("UPPER_RIGHT".to_string()),
            sprite_refs: Vec::new(),
            font: None,
            text: None,
            format: None,
            children: vec![synthetic_element(
                "background",
                "Background",
                400,
                0,
                32,
                32,
                Some("GFX_tiled_window_transparent"),
            )],
            raw: Value::Object(Vec::new()),
        };
        let rect = outliner_panel_rect(&shell, 1920, 1080);
        assert_eq!(rect.x(), 1560);
        assert_eq!(rect.y(), 117);
        assert_eq!(rect.w(), 360);
        assert_eq!(rect.h(), 723);
    }

    fn test_sprite(name: &str, no_of_frames: u32) -> SpriteDefinition {
        SpriteDefinition {
            kind: "spriteType".to_string(),
            name: name.to_string(),
            texture_files: vec![TextureSlot {
                key: "texturefile".to_string(),
                path: "gfx/interface/mapmode/test.dds".to_string(),
            }],
            no_of_frames,
            effect_file: None,
            size: None,
            border_size: None,
            source_file: "synthetic.gfx".to_string(),
            raw: Value::Object(Vec::new()),
        }
    }
}
