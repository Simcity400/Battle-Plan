use crate::assets::{normalize_rel_path, AssetResolver};
use crate::compose::{
    draw_image, draw_nine_slice, draw_rect_outline, draw_scaled, fill_rect,
    measure_to_pixels_for_parent, resolve_element_rect, ComposeOptions, ComposeOutput,
    unique_inline_sprite_definition, ComposeReport, Compositor, DebugRect, Rect, UiAssetCache,
};
use crate::dds::RgbaImage;
use crate::engine_text_layout::{
    engine_inline_icon_frame_index, layout_engine_text_with_icons, EngineTextBorder,
    EngineTextLayout, ResolvedInlineIconMetrics,
};
use crate::font::{blank_image, BitmapFont, RuntimeGlyphDraw, TextAlign};
use crate::gfx::{Size, SpriteRegistry};
use crate::gui::{parse_measure, GuiElement, MeasurePair};
use crate::gui_scene_util::{
    load_last_top_level_named_element_from_bytes, load_named_element_from_bytes, parse_layout_set,
    wrap_text_greedy,
};
use crate::loc::Localisation;
use crate::paradox_text::{scalar_i32, Value};
use crate::png::write_png;
use crate::tooltip_compositor::{
    compose_payload_from_facts, place_under_viewer_cursor, AdmittedTooltipSource,
    AuthoredClampOffsets, PlacementEvidence, ResolvedPlacement, TooltipAnchor, TooltipFamilyFacts,
    TooltipPayload, ViewerCursorPlacement,
};
use crate::topbar::{
    compose_topbar_at, topbar_debug_overlay, TopbarBinding,
};
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedTooltipChrome {
    pub root_name: String,
    pub source_file: String,
    pub texture_file: String,
    pub border: (usize, usize),
    pub font: String,
    pub base_text_color: [u8; 4],
    pub max_width: usize,
    pub max_height: usize,
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
    pub use_four_button_layout: Option<bool>,
}

impl Default for MapmodeBinding {
    fn default() -> Self {
        Self {
            selected_button: 0,
            use_four_button_layout: None,
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
    pub owner_root_name: String,
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
                use_four_button_layout: Some(true),
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
                owner_root_name: TOOLTIP_WINDOW_NAME.to_string(),
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

    let minimap = load_window_from_bytes(&resolver.read_file(&minimap_file)?,
        &minimap_file.rel_path,
        MINIMAP_WINDOW_NAME,
    )?;
    let mapmodes = load_window_from_bytes(&resolver.read_file(&mapmodes_file)?,
        &mapmodes_file.rel_path,
        MAPMODES_WINDOW_NAME,
    )?;
    let menubar_bytes = resolver.read_file(&menubar_file)?;
    let zoom_in =
        load_named_element_from_bytes(&menubar_bytes, &menubar_file.rel_path, "zoom_in")?;
    let zoom_out =
        load_named_element_from_bytes(&menubar_bytes, &menubar_file.rel_path, "zoom_out")?;
    let outliner_bytes = resolver.read_file(&outliner_file)?;
    let outliner_shell = load_window_from_bytes(&outliner_bytes,
        &outliner_file.rel_path,
        OUTLINER_WINDOW_NAME,
    )?;
    let outliner_row = load_window_from_bytes(&outliner_bytes,
        &outliner_file.rel_path,
        OUTLINER_ROW_WINDOW_NAME,
    )?;
    let tooltip_shell =
        load_named_element_from_bytes(&resolver.read_file(&core_file)?, &core_file.rel_path, TOOLTIP_WINDOW_NAME)?;

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

pub fn load_authenticated_tooltip_chrome(
    resolver: &AssetResolver,
    expected_root_name: &str,
) -> Result<AuthenticatedTooltipChrome> {
    load_authenticated_tooltip_chrome_for_language(resolver, expected_root_name, "l_english")
}

pub fn load_authenticated_tooltip_chrome_for_language(
    resolver: &AssetResolver,
    expected_root_name: &str,
    selected_language: &str,
) -> Result<AuthenticatedTooltipChrome> {
    if !selected_language.eq_ignore_ascii_case("l_english") {
        return Err(GameUiError::missing(format!(
            "XUI05 exact tooltip font-family consumption is not yet proven for \
             playback language {selected_language:?}"
        )));
    }
    let core_file = resolver.resolve_file(CORE_GUI)?;
    let element = load_last_top_level_named_element_from_bytes(&resolver.read_file(&core_file)?,
        &core_file.rel_path,
        expected_root_name,
    )?;
    if !element.element_type.eq_ignore_ascii_case("textBoxType") {
        return Err(GameUiError::parse(format!(
            "authored tooltip root {expected_root_name:?} in {CORE_GUI} is a {:?}, not a \
             textBoxType; the production route will not read tooltip chrome off a \
             same-named element of another type",
            element.element_type
        )));
    }
    let texture_file = element
        .raw
        .first_scalar_ci("textureFile")
        .map(normalize_rel_path)
        .ok_or_else(|| {
            GameUiError::missing(format!(
                "authored tooltip root {expected_root_name:?} in {CORE_GUI} declares no \
                 textureFile; the production route does not substitute one"
            ))
        })?;
    let border = strict_border_size(&element.raw).map_err(|error| {
        GameUiError::missing(format!(
            "authored tooltip root {expected_root_name:?} in {CORE_GUI}: {}; the production \
             route does not substitute an inset",
            error.message
        ))
    })?;
    let font = element.font.clone().ok_or_else(|| {
        GameUiError::missing(format!(
            "authored tooltip root {expected_root_name:?} in {CORE_GUI} declares no font; \
             the production route does not fall back to {TOOLTIP_FONT_FALLBACK}"
        ))
    })?;
    let max_width = strict_positive_authored_scalar(&element.raw, "maxWidth")?;
    let max_height = strict_positive_authored_scalar(&element.raw, "maxHeight")?;
    let base_text_color = legacy_authored_font_base_color(resolver, &font)?;
    Ok(AuthenticatedTooltipChrome {
        root_name: expected_root_name.to_string(),
        source_file: core_file.rel_path,
        texture_file,
        border,
        font,
        base_text_color,
        max_width,
        max_height,
    })
}

fn legacy_authored_font_base_color(resolver: &AssetResolver, font: &str) -> Result<[u8; 4]> {
    let core = resolver.resolve_file("interface/core.gfx")?;
    let bytes = resolver.read_file(&core)?;
    let document = crate::paradox_text::parse_bytes(&bytes);
    if !document.recoveries.is_empty() {
        return Err(GameUiError::parse("core.gfx font colour parse required recovery"));
    }
    let mut values = Vec::new();
    for container in document.root.values_ci("bitmapfonts") {
        for entry in container.fields() {
            if !entry
                .key
                .as_deref()
                .is_some_and(|kind| kind.eq_ignore_ascii_case("bitmapfont"))
                || entry.value.first_scalar_ci("name") != Some(font)
            {
                continue;
            }
            let raw = entry
                .value
                .first_scalar_ci("color")
                .ok_or_else(|| GameUiError::missing(format!("font {font:?} omits color")))?;
            let hex = raw.strip_prefix("0x").ok_or_else(|| {
                GameUiError::parse(format!("font {font:?} color is not 0xAARRGGBB"))
            })?;
            if hex.len() != 8 {
                return Err(GameUiError::parse(format!(
                    "font {font:?} color is not 0xAARRGGBB"
                )));
            }
            let argb = u32::from_str_radix(hex, 16)
                .map_err(|_| GameUiError::parse(format!("font {font:?} color is invalid")))?;
            values.push([
                ((argb >> 16) & 0xff) as u8,
                ((argb >> 8) & 0xff) as u8,
                (argb & 0xff) as u8,
                ((argb >> 24) & 0xff) as u8,
            ]);
        }
    }
    values.sort();
    values.dedup();
    match values.as_slice() {
        [color] => Ok(*color),
        _ => Err(GameUiError::missing(format!(
            "font {font:?} has {} effective base colors",
            values.len()
        ))),
    }
}

pub(crate) fn load_authenticated_tooltip_chrome_from_surface(
    surface: &crate::authored_role_index::AuthenticatedPanelSurface,
) -> Result<AuthenticatedTooltipChrome> {
    if surface.tooltip_shell().is_none() {
        return Err(GameUiError::missing(
            "XUI05 tooltip-shell capability is absent from the authenticated surface",
        ));
    }
    let element = surface
        .authored_element(&[])
        .ok_or_else(|| GameUiError::missing("XUI05 tooltip-shell root is absent"))?;
    if !element.element_type.eq_ignore_ascii_case("textBoxType") {
        return Err(GameUiError::parse(format!(
            "authenticated tooltip shell {:?} is {:?}, not textBoxType",
            element.name, element.element_type
        )));
    }
    let texture_file = element
        .raw
        .first_scalar_ci("textureFile")
        .map(normalize_rel_path)
        .ok_or_else(|| GameUiError::missing("authenticated tooltip shell has no textureFile"))?;
    let border = strict_border_size(&element.raw)?;
    let font = element
        .font
        .clone()
        .ok_or_else(|| GameUiError::missing("authenticated tooltip shell has no font"))?;
    let max_width = strict_positive_authored_scalar(&element.raw, "maxWidth")?;
    let max_height = strict_positive_authored_scalar(&element.raw, "maxHeight")?;
    let font_binding = surface
        .font_binding(&font)
        .map_err(GameUiError::missing)?
        .ok_or_else(|| {
            GameUiError::missing(format!(
                "authenticated tooltip shell has no exact binding for font {font:?}"
            ))
        })?;
    let base_text_color = font_binding
        .base_color
        .as_ref()
        .map(crate::authored_role_index::AuthoredPackedColorRole::rgba8)
        .ok_or_else(|| {
            GameUiError::missing(format!(
                "authenticated tooltip font {font:?} omits its base color"
            ))
        })?;
    Ok(AuthenticatedTooltipChrome {
        root_name: element.name.clone(),
        source_file: surface.source_content().normalized_game_relative_path.clone(),
        texture_file,
        border,
        font,
        base_text_color,
        max_width,
        max_height,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ComposedTooltipPlacementInput {
    pub raw_cursor_px: (i32, i32),
    pub gui_scale_f32_bits: (u32, u32),
    pub anchor: TooltipAnchor,
    pub viewport: (i32, i32),
    pub clamp: AuthoredClampOffsets,
    pub clamp_source_files: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreparedComposedTooltip {
    binding: TooltipBinding,
    payload: TooltipPayload,
    placement: ResolvedPlacement,
    chrome: AuthenticatedTooltipChrome,
    font: BitmapFont,
    text_layout: EngineTextLayout,
    inline_icons: Vec<PreparedTooltipInlineIcon>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PreparedTooltipInlineIcon {
    sprite: String,
    frame: u32,
    image: RgbaImage,
}

impl PreparedComposedTooltip {
    pub fn binding(&self) -> &TooltipBinding {
        &self.binding
    }

    pub fn payload(&self) -> &TooltipPayload {
        &self.payload
    }

    pub fn placement_evidence(&self) -> &PlacementEvidence {
        &self.placement.evidence
    }
}

pub(crate) fn prepare_composed_tooltip(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    localisation: &Localisation,
    tooltip_surface: &crate::authored_role_index::AuthenticatedPanelSurface,
    facts: TooltipFamilyFacts,
    placement_input: ComposedTooltipPlacementInput,
) -> Result<PreparedComposedTooltip> {
    let mut payload = compose_payload_from_facts(facts, localisation)
        .map_err(|error| GameUiError::missing(format!("XUI05 tooltip refused: {error}")))?;
    let chrome = load_authenticated_tooltip_chrome_from_surface(tooltip_surface)?;
    if chrome.root_name != payload.tooltip_root_name {
        return Err(GameUiError::missing(format!(
            "XUI05 payload shell {:?} differs from authenticated shell {:?}",
            payload.tooltip_root_name, chrome.root_name
        )));
    }
    let font_binding = tooltip_surface
        .font_binding(&chrome.font)
        .map_err(GameUiError::missing)?
        .ok_or_else(|| GameUiError::missing("XUI05 tooltip font binding disappeared"))?;
    let font = BitmapFont::load_from_authored_binding(resolver, font_binding)?;
    let resolved_rows = payload
        .rows
        .iter()
        .filter(|row| {
            matches!(
                row.availability,
                crate::tooltip_compositor::RowAvailability::Resolved
            )
        })
        .collect::<Vec<_>>();
    if resolved_rows.is_empty() {
        return Err(GameUiError::missing(
            "XUI05 composed payload contains no resolved row",
        ));
    }
    let text = resolved_rows
        .iter()
        .map(|row| row.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let border_x = i32::try_from(chrome.border.0)
        .map_err(|_| GameUiError::parse("authored borderSize.x overflows pixel space"))?;
    let border_y = i32::try_from(chrome.border.1)
        .map_err(|_| GameUiError::parse("authored borderSize.y overflows pixel space"))?;
    let (resolved_icon_metrics, inline_icons, icon_sources) =
        resolve_tooltip_inline_icons(resolver, registry, &font, &text)?;
    let max_width = i32::try_from(chrome.max_width)
        .map_err(|_| GameUiError::parse("authored maxWidth overflows pixel space"))?;
    let text_layout = layout_engine_text_with_icons(
        &font,
        &text,
        max_width,
        EngineTextBorder::Authored {
            x: border_x,
            y: border_y,
        },
        &resolved_icon_metrics,
    )
    .map_err(|error| GameUiError::missing(format!("XUI05 TEXT1 layout refused: {error}")))?;
    let shell_width = text_layout.total_width;
    let shell_height = text_layout.total_height;
    if shell_width <= 0 || shell_height <= 0 {
        return Err(GameUiError::parse(
            "XUI05 TEXT1 layout produced a non-positive tooltip extent",
        ));
    }
    let max_height = i32::try_from(chrome.max_height)
        .map_err(|_| GameUiError::parse("authored maxHeight overflows pixel space"))?;
    if shell_height > max_height {
        return Err(GameUiError::missing(format!(
            "XUI05 composed tooltip needs {shell_height}px but authored maxHeight is {max_height}px"
        )));
    }
    let selected_variant = font_binding
        .variants
        .get(font_binding.selected_variant_index)
        .ok_or_else(|| GameUiError::missing("XUI05 selected font variant is absent"))?;
    let mut asset_sources = vec![
        AdmittedTooltipSource::EffectiveAsset {
            role: "tooltip chrome GUI".into(),
            game_relative_path: chrome.source_file.clone(),
        },
        AdmittedTooltipSource::EffectiveAsset {
            role: "tooltip chrome texture".into(),
            game_relative_path: chrome.texture_file.clone(),
        },
        AdmittedTooltipSource::EffectiveAsset {
            role: "tooltip bitmap-font registration".into(),
            game_relative_path: selected_variant
                .registration_source_content
                .normalized_game_relative_path
                .clone(),
        },
        AdmittedTooltipSource::EffectiveAsset {
            role: "tooltip bitmap-text effect".into(),
            game_relative_path: font_binding
                .text_effect_content
                .normalized_game_relative_path
                .clone(),
        },
    ];
    for (index, descriptor) in selected_variant.ordered_descriptors.iter().enumerate() {
        asset_sources.push(AdmittedTooltipSource::EffectiveAsset {
            role: format!("tooltip BMFont descriptor {index}"),
            game_relative_path: descriptor
                .descriptor_content
                .normalized_game_relative_path
                .clone(),
        });
        asset_sources.extend(descriptor.ordered_pages.iter().map(|page| {
            AdmittedTooltipSource::EffectiveAsset {
                role: format!("tooltip BMFont descriptor {index} atlas page {}", page.page_id),
                game_relative_path: page.content.normalized_game_relative_path.clone(),
            }
        }));
    }
    if let Some(color) = &font_binding.base_color {
        asset_sources.push(AdmittedTooltipSource::EffectiveAsset {
            role: "tooltip bitmap-font base color".into(),
            game_relative_path: color.source_content.normalized_game_relative_path.clone(),
        });
    }
    for table in &font_binding.global_textcolor_tables {
        asset_sources.push(AdmittedTooltipSource::EffectiveAsset {
            role: "tooltip global section-colour table".into(),
            game_relative_path: table.source_content.normalized_game_relative_path.clone(),
        });
    }
    if let Some(table) = &font_binding.font_local_textcolors {
        asset_sources.push(AdmittedTooltipSource::EffectiveAsset {
            role: "tooltip font-local section-colour table".into(),
            game_relative_path: table.source_content.normalized_game_relative_path.clone(),
        });
    }
    for row in &mut payload.rows {
        if matches!(
            row.availability,
            crate::tooltip_compositor::RowAvailability::Resolved
        ) {
            row.admitted_sources.extend(icon_sources.iter().cloned());
            row.admitted_sources.extend(asset_sources.iter().cloned());
            for source in &placement_input.clamp_source_files {
                row.admitted_sources.push(AdmittedTooltipSource::EffectivePlacement { graphics_defines_file: source.clone() });
            }
        }
    }
    let placement = place_under_viewer_cursor(ViewerCursorPlacement {
        raw_cursor_px: placement_input.raw_cursor_px,
        gui_scale_f32_bits: placement_input.gui_scale_f32_bits,
        anchor: placement_input.anchor,
        viewport: placement_input.viewport,
        tooltip_extent: (shell_width, shell_height),
        clamp: placement_input.clamp,
    })
    .map_err(|error| GameUiError::missing(format!("XUI05 tooltip refused: {error}")))?;
    let width = usize::try_from(shell_width)
        .map_err(|_| GameUiError::parse("XUI05 composed width is negative"))?;
    let height = usize::try_from(shell_height)
        .map_err(|_| GameUiError::parse("XUI05 composed height is negative"))?;
    Ok(PreparedComposedTooltip {
        binding: TooltipBinding {
            x: placement.screen_rect.x,
            y: placement.screen_rect.y,
            width,
            height,
            text,
            owner_root_name: payload.tooltip_root_name.clone(),
        },
        payload,
        placement,
        chrome,
        font,
        text_layout,
        inline_icons,
    })
}

fn resolve_tooltip_inline_icons(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    font: &BitmapFont,
    text: &str,
) -> Result<(
    Vec<ResolvedInlineIconMetrics>,
    Vec<PreparedTooltipInlineIcon>,
    Vec<AdmittedTooltipSource>,
)> {
    let empty_colors = crate::markup::TextColorTable::default();
    let table = font.color_table.as_ref().unwrap_or(&empty_colors);
    let mut metrics = Vec::new();
    let mut prepared = Vec::new();
    let mut sources = Vec::new();
    for token in crate::markup::parse(text, table) {
        let (key, frame) = match token {
            crate::markup::Token::Icon { key, frame } => (key, frame),
            crate::markup::Token::Variable { key } => {
                return Err(GameUiError::missing(format!(
                    "XUI05 unresolved localisation variable ${key}$ reached TEXT1 layout"
                )))
            }
            crate::markup::Token::DataFunction { expr } => {
                return Err(GameUiError::missing(format!(
                    "XUI05 unresolved data function [{expr}] reached TEXT1 layout"
                )))
            }
            crate::markup::Token::Text { .. } => continue,
        };
        let metric_token = match frame {
            Some(frame) => format!("{key}|{frame}"),
            None => key.clone(),
        };
        let sprite_name = if key.starts_with("GFX_") {
            key.clone()
        } else {
            format!("GFX_{key}")
        };
        let sprite = unique_inline_sprite_definition(registry, &sprite_name).map_err(|error| {
            GameUiError::missing(format!(
                "XUI05 inline icon {key:?} cannot use {sprite_name:?}: {}",
                error.message
            ))
        })?;
        let decoded = resolver.decode_sprite(sprite)?;
        let frame = engine_inline_icon_frame_index(frame).ok_or_else(|| {
            GameUiError::parse(format!(
                "XUI05 inline icon {metric_token:?} uses invalid authored frame zero"
            ))
        })?;
        let image = decoded
            .frames
            .get(
                usize::try_from(frame)
                    .map_err(|_| GameUiError::parse("XUI05 inline-icon frame overflows usize"))?,
            )
            .ok_or_else(|| {
                GameUiError::missing(format!(
                    "XUI05 inline sprite {sprite_name:?} frame {frame} is outside 0..{}",
                    decoded.frames.len()
                ))
            })?
            .clone();
        let advance = i32::try_from(image.width)
            .map_err(|_| GameUiError::parse("XUI05 inline-icon width overflows i32"))?;
        let height = i32::try_from(image.height)
            .map_err(|_| GameUiError::parse("XUI05 inline-icon height overflows i32"))?;
        metrics.push(ResolvedInlineIconMetrics {
            token: metric_token,
            sprite: sprite_name.clone(),
            frame,
            advance,
            height,
        });
        sources.push(AdmittedTooltipSource::EffectiveSprite {
            sprite: sprite_name.clone(),
            definition_file: sprite.source_file.clone(),
            texture_file: decoded.texture.rel_path.clone(),
        });
        prepared.push(PreparedTooltipInlineIcon {
            sprite: sprite_name,
            frame,
            image,
        });
    }
    Ok((metrics, prepared, sources))
}

pub(crate) fn compose_prepared_tooltip_layer(
    resolver: &AssetResolver,
    prepared: &PreparedComposedTooltip,
    viewport_width: usize,
    viewport_height: usize,
) -> Result<RgbaImage> {
    let viewport_width_i32 = i32::try_from(viewport_width)
        .map_err(|_| GameUiError::parse("XUI05 viewport width overflows i32"))?;
    let viewport_height_i32 = i32::try_from(viewport_height)
        .map_err(|_| GameUiError::parse("XUI05 viewport height overflows i32"))?;
    let rect = Rect::new(
        prepared.binding.x,
        prepared.binding.y,
        i32::try_from(prepared.binding.width)
            .map_err(|_| GameUiError::parse("XUI05 binding width overflows i32"))?,
        i32::try_from(prepared.binding.height)
            .map_err(|_| GameUiError::parse("XUI05 binding height overflows i32"))?,
    );
    if rect.x() < 0
        || rect.y() < 0
        || rect.x().saturating_add(rect.w()) > viewport_width_i32
        || rect.y().saturating_add(rect.h()) > viewport_height_i32
    {
        return Err(GameUiError::missing(
            "XUI05 viewer-composed tooltip does not fit the admitted viewport",
        ));
    }
    let texture = resolver
        .decode_texture(&prepared.chrome.texture_file)?
        .image;
    let mut output = blank_image(viewport_width, viewport_height);
    let font = &prepared.font;
    let rich = font.colorize(&prepared.binding.text, prepared.chrome.base_text_color);
    for glyph in &prepared.text_layout.glyphs {
        let left = rect.x().saturating_add(glyph.draw_x);
        let top = rect.y().saturating_add(glyph.draw_y);
        let right = i64::from(left) + glyph.width as i64;
        let bottom = i64::from(top) + glyph.height as i64;
        if left < rect.x()
            || top < rect.y()
            || right > i64::from(rect.x().saturating_add(rect.w()))
            || bottom > i64::from(rect.y().saturating_add(rect.h()))
        {
            return Err(GameUiError::parse(format!(
                "XUI05 TEXT1 glyph {:?} escapes the composed tooltip extent",
                glyph.ch
            )));
        }
    }
    for (layout, icon) in prepared
        .text_layout
        .inline_icons
        .iter()
        .zip(&prepared.inline_icons)
    {
        if layout.sprite != icon.sprite || layout.frame != icon.frame {
            return Err(GameUiError::parse(
                "XUI05 inline-icon layout and decoded sprite order disagree",
            ));
        }
        let x = rect.x().saturating_add(layout.x);
        let y = rect
            .y()
            .saturating_add(layout.line_y)
            .saturating_sub(layout.engine_center_offset_y);
        let right = i64::from(x) + icon.image.width as i64;
        let bottom = i64::from(y) + icon.image.height as i64;
        if x < rect.x()
            || y < rect.y()
            || right > i64::from(rect.x().saturating_add(rect.w()))
            || bottom > i64::from(rect.y().saturating_add(rect.h()))
        {
            return Err(GameUiError::parse(format!(
                "XUI05 inline sprite {:?} escapes the composed tooltip extent",
                icon.sprite
            )));
        }
    }

    draw_nine_slice(
        &mut output,
        &texture,
        rect.x(),
        rect.y(),
        prepared.binding.width,
        prepared.binding.height,
        prepared.chrome.border.0,
        prepared.chrome.border.1,
    );
    for glyph in &prepared.text_layout.glyphs {
        let tint = rich
            .colors
            .get(glyph.visible_char_index)
            .copied()
            .unwrap_or(prepared.chrome.base_text_color);
        match font.draw_runtime_glyph(
            &mut output,
            glyph.ch as u32,
            rect.x().saturating_add(glyph.draw_x),
            rect.y().saturating_add(glyph.draw_y),
            tint,
            Some([rect.x(), rect.y(), rect.w(), rect.h()]),
        ) {
            RuntimeGlyphDraw::Drawn => {}
            RuntimeGlyphDraw::GlyphAbsent => {
                return Err(GameUiError::missing(format!(
                    "XUI05 authored font lost glyph {:?} after TEXT1 layout",
                    glyph.ch
                )))
            }
            RuntimeGlyphDraw::PageAbsent { page } => {
                return Err(GameUiError::missing(format!(
                    "XUI05 authored font lost atlas page {page} after TEXT1 layout"
                )))
            }
        }
    }
    for (layout, icon) in prepared
        .text_layout
        .inline_icons
        .iter()
        .zip(&prepared.inline_icons)
    {
        draw_image(
            &mut output,
            &icon.image,
            rect.x().saturating_add(layout.x),
            rect.y()
                .saturating_add(layout.line_y)
                .saturating_sub(layout.engine_center_offset_y),
        );
    }
    Ok(output)
}

pub fn compose_hud(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    binding: &HudBinding,
    width: usize,
    height: usize,
) -> Result<HudComposeResult> {
    let layouts = load_hud_layouts(resolver)?;
    let localisation = Localisation::load_english(resolver)?;
    let mut cache = UiAssetCache::default();
    let mut output = empty_output(width, height);
    let mut stats = HudSpriteStats::default();

    let topbar = compose_topbar_at(
        resolver,
        registry,
        &binding.topbar,
        &localisation,
        &mut cache,
        width,
        height,
    )?;
    draw_image(&mut output.image, &topbar.output.image, 0, 0);
    merge_output(&mut output, topbar.output);

    compose_minimap_into(
        resolver,
        registry,
        &layouts,
        &binding.mapmode,
        &binding.minimap,
        &localisation,
        &mut cache,
        &mut output,
        &mut stats,
    )?;
    compose_outliner_into(
        resolver,
        registry,
        &layouts,
        &binding.outliner,
        &localisation,
        &mut cache,
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
    let localisation = Localisation::load_english(resolver)?;
    let mut cache = UiAssetCache::default();
    let mut output = empty_output(width, height);
    let mut stats = HudSpriteStats::default();
    let mapmode = MapmodeBinding::default();
    compose_minimap_into(
        resolver,
        registry,
        &layouts,
        &mapmode,
        binding,
        &localisation,
        &mut cache,
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
    let localisation = Localisation::load_english(resolver)?;
    let mut cache = UiAssetCache::default();
    let mut output = empty_output(width, height);
    let mut stats = HudSpriteStats::default();
    compose_mapmodes_root_into(
        resolver,
        registry,
        &layouts.mapmodes,
        binding,
        &localisation,
        &mut cache,
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
    let localisation = Localisation::load_english(resolver)?;
    let mut cache = UiAssetCache::default();
    let mut output = empty_output(width, height);
    let mut stats = HudSpriteStats::default();
    compose_outliner_into(
        resolver,
        registry,
        &layouts,
        binding,
        &localisation,
        &mut cache,
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
    for name in MANUAL_HUD_SPRITES {
        names.insert((*name).to_string());
    }
    Ok(sprite_stats(resolver, registry, names))
}

const MANUAL_HUD_SPRITES: &[&str] = &[
    "GFX_tiled_window_transparent",
    "GFX_theatre_selection_background",
    "GFX_theatre_selection_frame",
    "GFX_theatre_army_shield",
    "GFX_theatre_task_force_shield",
    "GFX_theatre_air_force_shield",
    "GFX_topbar_fuel_bar",
    "GFX_production_output_icon",
];

fn compose_minimap_into(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    layouts: &HudLayouts,
    mapmode: &MapmodeBinding,
    binding: &MinimapBinding,
    localisation: &Localisation,
    cache: &mut UiAssetCache,
    output: &mut ComposeOutput,
    stats: &mut HudSpriteStats,
) -> Result<()> {
    compose_minimap_common(
        resolver,
        registry,
        &layouts.minimap,
        &layouts.mapmodes,
        &[&layouts.zoom_in, &layouts.zoom_out],
        mapmode,
        binding,
        localisation,
        cache,
        output,
        stats,
    )
}

#[allow(clippy::too_many_arguments)]
fn compose_minimap_common(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    minimap_root: &GuiElement,
    mapmodes_root: &GuiElement,
    zoom_roots: &[&GuiElement],
    mapmode: &MapmodeBinding,
    binding: &MinimapBinding,
    localisation: &Localisation,
    cache: &mut UiAssetCache,
    output: &mut ComposeOutput,
    stats: &mut HudSpriteStats,
) -> Result<()> {
    let mut options = localized_options(localisation, output.image.width, output.image.height);
    hide_unbound_text_elements(minimap_root, &mut options);
    if let Some(minimap) = &binding.image {
        let (w, h) = sprite_first_frame_size(resolver, registry, "GFX_minimap_world")?;
        options
            .sprite_overrides
            .insert("worldmap_bg".to_string(), scaled_image(minimap, w, h));
    }
    let minimap = compose_root(resolver, registry, cache, minimap_root, &options);
    let viewport = minimap
        .debug_rects
        .iter()
        .find(|rect| rect.name == "worldmap_bg")
        .cloned();
    draw_image(&mut output.image, &minimap.image, 0, 0);
    merge_output(output, minimap);

    compose_mapmodes_root_into(
        resolver,
        registry,
        mapmodes_root,
        mapmode,
        localisation,
        cache,
        output,
        stats,
    )?;

    if !zoom_roots.is_empty() {
        let zoom_options = localized_options(localisation, output.image.width, output.image.height);
        for zoom in zoom_roots {
            let zoom_output = compose_root(resolver, registry, cache, zoom, &zoom_options);
            draw_image(&mut output.image, &zoom_output.image, 0, 0);
            merge_output(output, zoom_output);
        }
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
            child_path: Vec::new(),
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
            std::iter::once(minimap_root).chain(zoom_roots.iter().copied()),
        ),
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn compose_mapmodes_root_into(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    mapmodes_root: &GuiElement,
    binding: &MapmodeBinding,
    localisation: &Localisation,
    cache: &mut UiAssetCache,
    output: &mut ComposeOutput,
    stats: &mut HudSpriteStats,
) -> Result<()> {
    let mut options = localized_options(localisation, output.image.width, output.image.height);
    apply_mapmode_binding(mapmodes_root, registry, binding, &mut options);
    let mapmodes = compose_root(resolver, registry, cache, mapmodes_root, &options);
    draw_image(&mut output.image, &mapmodes.image, 0, 0);
    merge_output(output, mapmodes);
    *stats = merge_sprite_stats(
        stats,
        &sprite_stats_for_roots(resolver, registry, [mapmodes_root]),
    );
    Ok(())
}

fn apply_mapmode_binding(
    root: &GuiElement,
    registry: &SpriteRegistry,
    binding: &MapmodeBinding,
    options: &mut ComposeOptions,
) {
    hide_unbound_text_elements(root, options);
    options
        .hidden_elements
        .insert("social_view_interface_window".to_string());

    match binding.use_four_button_layout {
        Some(true) => {
            options
                .hidden_elements
                .insert("3_button_mapmode".to_string());
        }
        Some(false) => {
            options
                .hidden_elements
                .insert("4_button_mapmode".to_string());
        }
        None => {
            options
                .hidden_elements
                .insert("3_button_mapmode".to_string());
            options
                .hidden_elements
                .insert("4_button_mapmode".to_string());
        }
    }

    let selected = (1..=11)
        .contains(&binding.selected_button)
        .then_some(binding.selected_button);
    for button in 1..=11u8 {
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
    for inert in [
        "mapmode_button_12",
        "mapmode_button_12_s",
        "mapmode_button_12_s_bg",
    ] {
        options.hidden_elements.insert(inert.to_string());
    }
    apply_mapmode_frame_overrides(root, registry, options);
}

fn hide_unbound_text_elements(element: &GuiElement, options: &mut ComposeOptions) {
    if element
        .element_type
        .eq_ignore_ascii_case("instantTextBoxType")
    {
        options.hidden_elements.insert(element.name.clone());
    }
    for child in &element.children {
        hide_unbound_text_elements(child, options);
    }
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

#[allow(clippy::too_many_arguments)]
fn compose_outliner_into(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    layouts: &HudLayouts,
    binding: &OutlinerBinding,
    localisation: &Localisation,
    cache: &mut UiAssetCache,
    output: &mut ComposeOutput,
    stats: &mut HudSpriteStats,
) -> Result<()> {
    let shell = compose_root(
        resolver,
        registry,
        cache,
        &layouts.outliner_shell,
        &localized_options(localisation, output.image.width, output.image.height),
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
        child_path: Vec::new(),
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

    let mut names = BTreeSet::new();
    layouts.outliner_shell.collect_sprite_refs(&mut names);
    layouts.outliner_row.collect_sprite_refs(&mut names);
    for name in MANUAL_HUD_SPRITES {
        names.insert((*name).to_string());
    }
    for row in &binding.rows {
        names.insert(row.icon_sprite.clone());
    }
    *stats = merge_sprite_stats(stats, &sprite_stats(resolver, registry, names));
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
    let text_rect = tooltip_text_rect(rect, border_x, border_y);
    let wrapped = wrap_text_greedy(&binding.text, text_rect.w().max(8), |text| {
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
        child_path: Vec::new(),
        x: rect.x(),
        y: rect.y(),
        w: rect.w(),
        h: rect.h(),
        depth: 1,
    });
    output.debug_rects.push(DebugRect {
        name: "tooltip_text".to_string(),
        element_type: "instantTextBoxType".to_string(),
        child_path: vec![0],
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
            child_path: vec![u32::try_from(idx)
                .expect("outliner row count cannot exceed address space")],
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
    cache: &mut UiAssetCache,
    root: &GuiElement,
    options: &ComposeOptions,
) -> ComposeOutput {
    let mut compositor = Compositor::with_cache(resolver, registry, cache);
    compositor.compose(root, options)
}

fn empty_output(width: usize, height: usize) -> ComposeOutput {
    ComposeOutput {
        image: blank_image(width, height),
        report: ComposeReport::default(),
        debug_rects: Vec::new(),
    }
}

fn localized_options(localisation: &Localisation, width: usize, height: usize) -> ComposeOptions {
    ComposeOptions::new(width, height).with_localisation(localisation)
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
    dst.localisation_misses.absorb(src.localisation_misses);
    for (key, source) in src.localisation_sources_used {
        match dst.localisation_sources_used.entry(key) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(source);
            }
            std::collections::btree_map::Entry::Occupied(entry) => {
                debug_assert_eq!(
                    entry.get(),
                    &source,
                    "one localisation key cannot have two admitted definitions"
                );
            }
        }
    }
    dst.unresolved_text_inputs
        .extend(src.unresolved_text_inputs);
}

fn load_window_from_bytes(bytes: &[u8], rel: &str, name: &str) -> Result<GuiElement> {
    parse_layout_set(bytes, rel)?
        .documents
        .iter()
        .flat_map(|document| document.roots.iter())
        .rev()
        .find(|element| {
            element.name.eq_ignore_ascii_case(name)
                && (element
                    .element_type
                    .eq_ignore_ascii_case("containerWindowType")
                    || element.element_type.eq_ignore_ascii_case("windowType"))
        })
        .cloned()
        .ok_or_else(|| GameUiError::missing(format!("{rel}: window {name} not found")))
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

fn strict_border_size(value: &Value) -> Result<(usize, usize)> {
    let border = value
        .first_value_ci("borderSize")
        .ok_or_else(|| GameUiError::missing("borderSize is absent"))?;
    let component = |name: &str| -> Result<usize> {
        let raw = border
            .first_value_ci(name)
            .and_then(scalar_i32)
            .ok_or_else(|| {
                GameUiError::missing(format!("borderSize declares no parseable {name}"))
            })?;
        usize::try_from(raw).map_err(|_| {
            GameUiError::parse(format!(
                "borderSize {name} is {raw}; a negative inset is not clamped to zero here"
            ))
        })
    };
    Ok((component("x")?, component("y")?))
}

fn strict_positive_authored_scalar(value: &Value, name: &str) -> Result<usize> {
    let raw = value
        .first_value_ci(name)
        .and_then(scalar_i32)
        .ok_or_else(|| GameUiError::missing(format!("{name} is absent or unparseable")))?;
    let parsed = usize::try_from(raw).map_err(|_| {
        GameUiError::parse(format!(
            "{name} is {raw}; a negative authored quantity is not clamped"
        ))
    })?;
    if parsed == 0 {
        return Err(GameUiError::parse(format!(
            "{name} is zero; the production tooltip cannot form an extent"
        )));
    }
    Ok(parsed)
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

fn tooltip_text_rect(shell: Rect, border_x: usize, border_y: usize) -> Rect {
    let border_x = border_x as i32;
    let border_y = border_y as i32;
    Rect::new(
        shell.x() + border_x,
        shell.y() + border_y,
        shell.w() - border_x * 2,
        shell.h() - border_y * 2,
    )
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

    fn unique_tooltip_test_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "game_ui_tip2_{name}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock is after Unix epoch")
                .as_nanos()
        ))
    }

    #[test]
    fn production_tooltip_border_read_refuses_where_the_probe_read_defaults() {
        let parse = |text: &str| crate::paradox_text::parse_bytes(text.as_bytes()).root;

        let ok = parse("borderSize = { x = 18 y = 18 }");
        assert_eq!(strict_border_size(&ok).unwrap(), (18, 18));
        assert_eq!(border_size(&ok), Some((18, 18)));

        let partial = parse("borderSize = { x = 18 }");
        assert_eq!(border_size(&partial), Some((18, 0)));
        let error = strict_border_size(&partial).expect_err("a missing component must refuse");
        assert!(
            error.message.contains("no parseable y"),
            "{}",
            error.message
        );

        let negative = parse("borderSize = { x = -4 y = 18 }");
        assert_eq!(border_size(&negative), Some((0, 18)));
        let error = strict_border_size(&negative).expect_err("a negative inset must refuse");
        assert!(
            error.message.contains("not clamped to zero"),
            "{}",
            error.message
        );

        assert!(strict_border_size(&parse("font = \"cg_16b\"")).is_err());
    }

    #[test]
    fn composed_chrome_discovers_a_mod_renamed_root_and_all_authored_extents() {
        let root = unique_tooltip_test_root("renamed_root");
        let interface = root.join("interface");
        std::fs::create_dir_all(&interface).expect("create synthetic interface directory");
        std::fs::write(
            interface.join("core.gui"),
            "guiTypes = { textBoxType = { name = \"ModRenamedTip\" \
             textureFile = \"gfx/interface/mod_tip.tga\" font = \"mod_tip_font\" \
             borderSize = { x = 7 y = 9 } maxWidth = 321 maxHeight = 123 } }",
        )
        .expect("write synthetic effective core.gui");
        std::fs::write(
            interface.join("core.gfx"),
            "bitmapfonts = { bitmapfont = { name = \"mod_tip_font\" \
             path = \"gfx/fonts/mod_tip_font\" color = 0xffffffff } }",
        )
        .expect("write synthetic effective core.gfx");
        let resolver = AssetResolver::new(vec![root.clone()]);
        let chrome = load_authenticated_tooltip_chrome(&resolver, "ModRenamedTip")
            .expect("the authored renamed root is discovered");
        assert_eq!(chrome.root_name, "ModRenamedTip");
        assert_eq!(chrome.border, (7, 9));
        assert_eq!(chrome.max_width, 321);
        assert_eq!(chrome.max_height, 123);
        assert_eq!(chrome.font, "mod_tip_font");
        assert_eq!(chrome.texture_file, "gfx/interface/mod_tip.tga");
        portable_test_fs_retry::remove_dir_all(&root).expect("remove synthetic tooltip fixture");
    }

    #[test]
    fn composed_chrome_refuses_a_missing_authored_max_width() {
        let root = unique_tooltip_test_root("missing_max_width");
        let interface = root.join("interface");
        std::fs::create_dir_all(&interface).expect("create synthetic interface directory");
        std::fs::write(
            interface.join("core.gui"),
            "guiTypes = { textBoxType = { name = \"ModRenamedTip\" \
             textureFile = \"gfx/interface/mod_tip.tga\" font = \"mod_tip_font\" \
             borderSize = { x = 7 y = 9 } maxHeight = 123 } }",
        )
        .expect("write synthetic effective core.gui");
        let resolver = AssetResolver::new(vec![root.clone()]);
        let error = load_authenticated_tooltip_chrome(&resolver, "ModRenamedTip")
            .expect_err("maxWidth has no default");
        assert_eq!(
            error,
            GameUiError::missing("maxWidth is absent or unparseable")
        );
        portable_test_fs_retry::remove_dir_all(&root).expect("remove synthetic tooltip fixture");
    }

    #[test]
    fn composed_chrome_refuses_an_unproven_playback_language_before_loading_assets() {
        let error = load_authenticated_tooltip_chrome_for_language(
            &AssetResolver::new(Vec::new()),
            TOOLTIP_WINDOW_NAME,
            "l_japanese",
        )
        .expect_err("XUI05 may not guess a non-English font-family consumption law");
        assert!(
            error
                .message
                .contains("font-family consumption is not yet proven"),
            "{error}"
        );
    }

    #[test]
    fn markup_drops_icon_variable_and_function_tokens_from_the_glyph_stream() {
        let table = crate::markup::TextColorTable::default();
        for blob in ["A$X$B", "A\u{00a3}icon B", "A[GetName]B"] {
            let colorized =
                crate::font::colorize_with_table(blob, [1, 2, 3, 255], Some(&table)).text;
            let stripped = crate::font::strip_color_codes(blob);
            assert_eq!(
                stripped, blob,
                "strip_color_codes must leave {blob:?} intact"
            );
            assert_ne!(
                colorized, stripped,
                "colorize must be shown to drop {blob:?}, or the route's refusal is pointless"
            );
        }
        let coloured = "\u{00a7}GReady\u{00a7}!";
        assert_eq!(
            crate::font::colorize_with_table(coloured, [1, 2, 3, 255], Some(&table)).text,
            crate::font::strip_color_codes(coloured)
        );
    }

    #[test]
    #[ignore = "production verification supplies BATTLE_PLAN_GAME_ROOT"]
    fn installed_tooltip_font_declares_the_line_height_the_engine_advances_by() {
        let root = std::path::PathBuf::from(
            std::env::var("BATTLE_PLAN_GAME_ROOT").or_else(|_| std::env::var("HOI4_GAME_ROOT")).expect("BATTLE_PLAN_GAME_ROOT must be set for this gate"),
        );
        assert!(
            root.is_dir(),
            "BATTLE_PLAN_GAME_ROOT {} is not a directory",
            root.display()
        );
        let resolver = AssetResolver::new(vec![root]);
        let chrome = load_authenticated_tooltip_chrome(&resolver, TOOLTIP_WINDOW_NAME)
            .expect("authored ToolTip chrome loads from the vanilla install");
        let font = BitmapFont::load(&resolver, &chrome.font)
            .expect("authored tooltip font loads");
        assert_eq!(
            font.name.to_ascii_lowercase(),
            "cg_16b",
            "the authored ToolTip font is the one this derivation was taken against"
        );
        assert_eq!(
            font.line_height, 16,
            "the shipped cg_16b.fnt declares lineHeight=16, which is FONT+0xC00 and the \
             engine's per-line advance; `engine_text_layout::layout_engine_text` and \
             `BitmapFont::layout_text` advance by this value and floor nothing"
        );
        assert_eq!(
            chrome.border,
            (18, 18),
            "the authored borderSize is the derived glyph origin"
        );
    }

    fn production_source() -> String {
        let source = include_str!("hud.rs").replace("\r\n", "\n");
        let marker = ["#[cfg(", "test)]"].concat();
        let cut = source
            .rfind(&marker)
            .expect("this module has a test module to cut off");
        source[..cut].to_string()
    }

    #[test]
    fn the_hud_production_source_slice_actually_excludes_the_tests() {
        let production = production_source();
        assert!(production.len() < include_str!("hud.rs").len());
        assert!(!production.contains('\r'));
        assert!(
            !production.contains(&["mod ", "tests {"].concat()),
            "the cut landed after the test module opens"
        );
        let test_only = [
            "the_hud_production_source_slice_",
            "actually_excludes_the_tests",
        ]
        .concat();
        assert!(!production.contains(&test_only));
        assert!(production.contains("pub(crate) fn compose_prepared_tooltip_layer("));
        assert!(production.contains(&["\npub fn ", "compose_hud("].concat()));
    }

    ///   the inline-sprite bounds check alone would otherwise still pass.
    #[test]
    fn probe_tooltip_line_colour_is_not_the_production_law() {
        let production = ComposeOptions::new(1920, 1080).text_color;
        assert_eq!(tooltip_line_color(1, "body"), production);
        assert_ne!(
            tooltip_line_color(0, "header"),
            production,
            "the probe draw still classifies line 0; the product route must not use it"
        );
        let hud_source = production_source();
        let product = hud_source
            .split_once("pub(crate) fn compose_prepared_tooltip_layer(")
            .unwrap()
            .1
            .split_once("\npub fn compose_hud(")
            .unwrap()
            .0;
        for forbidden in [
            "tooltip_line_color",
            "wrap_text_greedy",
            "TOOLTIP_FONT_FALLBACK",
        ] {
            assert!(
                !product.contains(forbidden),
                "production tooltip draw must not use {forbidden}"
            );
        }
        for (arm, message) in [
            (
                "RuntimeGlyphDraw::GlyphAbsent =>",
                "XUI05 authored font lost glyph {:?} after TEXT1 layout",
            ),
            (
                "RuntimeGlyphDraw::PageAbsent { page } =>",
                "XUI05 authored font lost atlas page {page} after TEXT1 layout",
            ),
        ] {
            let arm_at = product
                .find(arm)
                .unwrap_or_else(|| panic!("the {arm} refusal arm"));
            let message_at = product[arm_at..]
                .find(message)
                .unwrap_or_else(|| panic!("the message belonging to {arm}"));
            let between = &product[arm_at..arm_at + message_at];
            assert!(
                between.contains("return Err(GameUiError::missing(format!("),
                "{arm} must REFUSE with its message rather than report it and carry on"
            );
        }
        assert!(
            !product.contains("font.draw_text("),
            "the shipping tooltip route must not re-enter the blob-level draw"
        );
        assert_eq!(
            product.matches("font.colorize(").count(),
            1,
            "markup colour state must be established at exactly one site"
        );
        assert!(
            product.contains("font.colorize(&prepared.binding.text, prepared.chrome.base_text_color)"),
            "the single colourisation must take the entire recorded blob"
        );
        let colourise_at = product
            .find("font.colorize(")
            .expect("the colourisation site");
        let first_glyph_loop = product
            .find("for glyph in &prepared.text_layout.glyphs")
            .expect("the glyph loop");
        assert!(
            colourise_at < first_glyph_loop,
            "the blob must be colourised before the glyph loop, not inside it"
        );
        assert!(
            product.contains(".get(glyph.visible_char_index)"),
            "each glyph must take its tint from the precomputed per-character table"
        );
        assert!(product.contains("XUI05 TEXT1 glyph {:?} escapes the composed tooltip extent"));
        assert!(product.contains("XUI05 inline sprite {:?} escapes the composed tooltip extent"));
        let validate_source = include_str!("production_renderer.rs").replace("\r\n", "\n");
        let validate = validate_source
            .split_once("pub(crate) fn validate_complete_compose_report(")
            .unwrap()
            .1
            .split_once("\n}\n")
            .unwrap()
            .0;
        assert!(validate.contains("missing_glyphs"));
    }

    #[test]
    fn lower_right_view_rect_clamps_to_minimap_viewport() {
        let viewport = DebugRect {
            name: "worldmap_bg".to_string(),
            element_type: "iconType".to_string(),
            child_path: Vec::new(),
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
        let wrapped = wrap_text_greedy(
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
    fn tooltip_text_rect_uses_authored_border_size_as_glyph_origin() {
        let shell = Rect::new(100, 50, 300, 120);
        let text = tooltip_text_rect(shell, 18, 18);

        assert_eq!(text, Rect::new(118, 68, 264, 84));
        assert_eq!((text.x() - shell.x(), text.y() - shell.y()), (18, 18));
    }

    #[test]
    fn mapmode_binding_selects_one_twin_and_sets_sprite_frames() {
        let mut registry = SpriteRegistry::new();
        registry.insert(test_sprite("GFX_mapmode_buttons_deselected_small", 18));
        registry.insert(test_sprite("GFX_mapmode_buttons_selected_small", 18));
        let mut friends_online = synthetic_element(
            "instantTextBoxType",
            "friends_online",
            0,
            0,
            30,
            16,
            None,
        );
        friends_online.text = Some("0".to_string());
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
                friends_online,
            ],
            raw: Value::Object(Vec::new()),
        };
        let mut options = ComposeOptions::new(1920, 1080);
        apply_mapmode_binding(
            &root,
            &registry,
            &MapmodeBinding {
                selected_button: 5,
                use_four_button_layout: Some(true),
            },
            &mut options,
        );

        assert!(options.hidden_elements.contains("3_button_mapmode"));
        assert!(options.hidden_elements.contains("mapmode_button_5"));
        assert!(!options.hidden_elements.contains("mapmode_button_5_s"));
        assert!(!options.hidden_elements.contains("mapmode_button_5_s_bg"));
        assert!(options.hidden_elements.contains("mapmode_button_6_s"));
        assert!(options.hidden_elements.contains("friends_online"));
        assert!(options
            .hidden_elements
            .contains("social_view_interface_window"));
        assert_eq!(options.frame_overrides.get("mapmode_button_5"), Some(&4));
        assert_eq!(options.frame_overrides.get("mapmode_button_5_s"), Some(&4));
        assert_eq!(mapmode_frame_for_button(11, 18), 10);
    }

    #[test]
    fn absent_mapmode_runtime_state_selects_neither_layout_nor_button() {
        let root = synthetic_element("containerWindowType", "root", 0, 0, 20, 20, None);
        let registry = SpriteRegistry::new();
        let mut options = ComposeOptions::new(1920, 1080);
        apply_mapmode_binding(&root, &registry, &MapmodeBinding::default(), &mut options);
        assert!(options.hidden_elements.contains("3_button_mapmode"));
        assert!(options.hidden_elements.contains("4_button_mapmode"));
        for button in 1..=11u8 {
            assert!(!options
                .hidden_elements
                .contains(&format!("mapmode_button_{button}")));
            assert!(options
                .hidden_elements
                .contains(&format!("mapmode_button_{button}_s")));
            assert!(options
                .hidden_elements
                .contains(&format!("mapmode_button_{button}_s_bg")));
        }
        for inert in [
            "mapmode_button_12",
            "mapmode_button_12_s",
            "mapmode_button_12_s_bg",
        ] {
            assert!(options.hidden_elements.contains(inert));
        }
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
