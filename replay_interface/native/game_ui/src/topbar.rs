use crate::assets::AssetResolver;
use crate::compose::{
    draw_image, draw_scaled, draw_scaled_crop, resolve_element_rect, ComposeOptions, ComposeOutput,
    Compositor, DebugRect, Orientation, Rect,
};
use crate::dds::RgbaImage;
use crate::font::{blank_image, BitmapFont};
use crate::gfx::SpriteRegistry;
use crate::gui::{GuiElement, LayoutSet};
use crate::loc::Localisation;
use crate::paradox_text::parse_file;
use crate::png::write_png;
use crate::{GameUiError, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const TOPBAR_GUI: &str = "interface/topbar.gui";
pub const TOPBAR_WINDOW_NAME: &str = "topbar";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TopbarBinding {
    pub country_tag: Option<String>,
    pub country_ideology: Option<String>,
    /// 1936 `set_cosmetic_tag` override (e.g. `RAJ_UK`), if any. Takes
    /// precedence over the base tag for flag selection.
    pub cosmetic_tag: Option<String>,
    pub flag_texture: Option<String>,
    pub political_power: Option<String>,
    pub manpower: Option<String>,
    pub stability_percent: Option<String>,
    pub war_support_percent: Option<String>,
    pub command_power: Option<String>,
    pub army_experience: Option<String>,
    pub navy_experience: Option<String>,
    pub air_experience: Option<String>,
    pub nukes: Option<String>,
    pub factory_counts: Option<FactoryCounts>,
    pub factory_text: Option<String>,
    pub industrial_ratio: Option<f32>,
    pub convoys: Option<String>,
    pub convoys_ratio: Option<f32>,
    pub fuel: Option<String>,
    pub fuel_ratio: Option<f32>,
    pub supply: Option<String>,
    pub supply_ratio: Option<f32>,
    pub world_tension_percent: Option<String>,
    pub world_tension: Option<f32>,
    pub date_time: Option<String>,
    pub speed: Option<u8>,
    pub army_xp_ratio: Option<f32>,
    pub navy_xp_ratio: Option<f32>,
    pub air_xp_ratio: Option<f32>,
    pub initiative: Option<String>,
    pub decision_timeout_items: Option<String>,
    pub decision_available_items: Option<String>,
    pub intel_prepared_or_completed: Option<String>,
    pub intel_available: Option<String>,
    pub prototype_rewards: Option<String>,
    pub non_delivering_contracts: Option<String>,
    pub ongoing_contracts: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FactoryCounts {
    pub civilian: u32,
    pub military: u32,
    pub naval: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindingField {
    pub field: &'static str,
    pub gui_element: &'static str,
    pub default: &'static str,
    pub notes: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TopbarLayout {
    pub source_file: String,
    pub window_name: String,
    pub root: GuiElement,
    pub viewport_width: usize,
    pub viewport_height: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TopbarComposeResult {
    pub layout: TopbarLayout,
    pub output: ComposeOutput,
    pub flag_texture_used: Option<String>,
}

impl TopbarBinding {
    pub fn sample_ire() -> Self {
        Self {
            country_tag: Some("IRE".to_string()),
            country_ideology: Some("democratic".to_string()),
            political_power: Some("2000".to_string()),
            manpower: Some(format_scaled_short(66_100)),
            stability_percent: Some(format_percent(0.74)),
            war_support_percent: Some(format_percent(1.0)),
            command_power: Some("100".to_string()),
            army_experience: Some("165".to_string()),
            navy_experience: Some("0".to_string()),
            air_experience: Some("0".to_string()),
            nukes: Some("0".to_string()),
            factory_text: Some("11".to_string()),
            industrial_ratio: Some(1.0),
            convoys: Some("21".to_string()),
            convoys_ratio: Some(1.0),
            fuel: Some(format_percent(1.0)),
            fuel_ratio: Some(1.0),
            supply: Some(format!("\u{00a7}G{}\u{00a7}!", "Full")),
            supply_ratio: Some(1.0),
            world_tension_percent: Some(format_percent(1.0)),
            world_tension: Some(1.0),
            date_time: Some(format_topbar_datetime(2, 1, 7, 1944)),
            speed: Some(1),
            army_xp_ratio: Some(0.65),
            navy_xp_ratio: Some(0.25),
            air_xp_ratio: Some(0.45),
            initiative: Some("10".to_string()),
            decision_timeout_items: Some("10".to_string()),
            decision_available_items: Some("0".to_string()),
            intel_prepared_or_completed: Some("0".to_string()),
            intel_available: Some("0".to_string()),
            prototype_rewards: Some("0".to_string()),
            non_delivering_contracts: Some("0".to_string()),
            ongoing_contracts: Some("0".to_string()),
            ..Self::default()
        }
    }
}

impl FactoryCounts {
    fn to_topbar_text(&self) -> String {
        format!("{}/{}/{}", self.civilian, self.military, self.naval)
    }
}

pub fn format_topbar_datetime(hour: u8, day: u8, month: u8, year: i32) -> String {
    let month = match month {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        12 => "Dec",
        _ => "???",
    };
    format!("{}:00, {} {}, {}", hour, day, month, year)
}

pub fn format_percent(value: f32) -> String {
    format!("{}%", (value.clamp(0.0, 1.0) * 100.0).round() as i32)
}

pub fn format_scaled_short(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{:.2}M", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.2}K", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

pub fn frame_from_fraction(value: f32, frame_count: usize) -> usize {
    if frame_count <= 1 {
        return 0;
    }
    ((value.clamp(0.0, 1.0) * (frame_count - 1) as f32).round() as usize).min(frame_count - 1)
}

fn parse_percent(value: &str) -> Option<f32> {
    let number = value.trim().trim_end_matches('%').parse::<f32>().ok()?;
    Some((number / 100.0).clamp(0.0, 1.0))
}

fn draw_overview_xp_bars(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    binding: &TopbarBinding,
    output: &mut ComposeOutput,
) -> Result<()> {
    let Some(sprite) = registry.get("GFX_topbar_fuel_bar") else {
        return Ok(());
    };
    let Some(fg_rel) = sprite.texture_files.first().map(|slot| slot.path.as_str()) else {
        return Ok(());
    };
    let bg_rel = sprite
        .texture_files
        .get(1)
        .map(|slot| slot.path.as_str())
        .unwrap_or(fg_rel);
    let fg = resolver.decode_texture(fg_rel)?.image;
    let bg = resolver.decode_texture(bg_rel)?.image;
    let (natural_w, natural_h) = sprite
        .size
        .as_ref()
        .and_then(|size| {
            Some((
                size.x.parse::<f32>().ok()?.round().max(1.0) as usize,
                size.y.parse::<f32>().ok()?.round().max(1.0) as usize,
            ))
        })
        .unwrap_or((fg.width, fg.height));
    for (name, progress) in [
        ("army_button", binding.army_xp_ratio),
        ("navy_button", binding.navy_xp_ratio),
        ("air_button", binding.air_xp_ratio),
    ] {
        let Some(progress) = progress else {
            continue;
        };
        let Some(rect) = output.debug_rects.iter().find(|rect| rect.name == name) else {
            continue;
        };
        if rect.w <= 0 || rect.h <= 0 || natural_w == 0 || natural_h == 0 {
            continue;
        }
        let rect_w = rect.w.max(1) as usize;
        let rect_h = rect.h.max(1) as usize;
        let margin = natural_h.max(2) / 2;
        let target_w = natural_w.min(rect_w.saturating_sub(margin).max(1));
        let target_h = natural_h.min(rect_h.max(1));
        let x = rect.x + ((rect_w.saturating_sub(target_w)) / 2) as i32;
        let y = rect.y + rect_h.saturating_sub(target_h + margin / 2) as i32;
        draw_scaled(&mut output.image, &bg, x, y, target_w, target_h);
        let visible_w = ((target_w as f32) * progress.clamp(0.0, 1.0)).round() as usize;
        if visible_w > 0 {
            draw_scaled_crop(&mut output.image, &fg, x, y, target_w, target_h, visible_w);
        }
    }
    Ok(())
}

pub fn load_topbar_layout(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
) -> Result<TopbarLayout> {
    let resolved = resolver.resolve_file(TOPBAR_GUI)?;
    let doc = parse_file(&resolved.path)?;
    let mut layouts = LayoutSet::default();
    layouts.add_document(&resolved.rel_path, &doc);
    let root = layouts
        .documents
        .iter()
        .flat_map(|doc| doc.roots.iter())
        .find(|root| {
            root.name.eq_ignore_ascii_case(TOPBAR_WINDOW_NAME)
                && root
                    .element_type
                    .eq_ignore_ascii_case("containerWindowType")
        })
        .cloned()
        .ok_or_else(|| GameUiError::missing("topbar containerWindowType named topbar not found"))?;
    let (viewport_width, viewport_height) = topbar_native_size(resolver, registry)?;
    Ok(TopbarLayout {
        source_file: resolved.rel_path,
        window_name: TOPBAR_WINDOW_NAME.to_string(),
        root,
        viewport_width,
        viewport_height,
    })
}

/// A topbar element rect resolved purely from the `.gui` layout fields
/// (position / size / orientation) through the decompiled engine anchor law in
/// [`resolve_element_rect`]. Natural sprite sizes are deliberately treated as
/// zero, so every value here is a pure function of `topbar.gui` and the layout
/// rule — no texture decode is involved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedTopbarRect {
    pub path: String,
    pub name: String,
    pub element_type: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub orientation: Orientation,
    /// Human-readable note of which `.gui` fields drove this rect.
    pub drivers: String,
}

/// Resolve every topbar element's screen rect at a fixed viewport using ONLY
/// the parsed `.gui` fields and the engine-exact orientation/anchor law.
pub fn resolve_topbar_rects_pure(
    layout: &TopbarLayout,
    viewport_w: usize,
    viewport_h: usize,
) -> Vec<ResolvedTopbarRect> {
    let options =
        ComposeOptions::new(viewport_w, viewport_h).with_root_size(viewport_w, viewport_h);
    let root_parent = Rect::new(0, 0, viewport_w as i32, viewport_h as i32);
    let mut out = Vec::new();
    walk_pure(&layout.root, root_parent, "", &options, &mut out);
    out
}

fn walk_pure(
    element: &GuiElement,
    parent: Rect,
    parent_path: &str,
    options: &ComposeOptions,
    out: &mut Vec<ResolvedTopbarRect>,
) {
    let rect = resolve_element_rect(element, parent, (0, 0), options);
    let orientation = Orientation::parse(element.orientation.as_deref());
    let path = if parent_path.is_empty() {
        element.name.clone()
    } else {
        format!("{parent_path}/{}", element.name)
    };
    let mut drivers = Vec::new();
    if let Some(pos) = &element.position {
        drivers.push(format!("position=({},{})", pos.x.raw, pos.y.raw));
    }
    if let Some(size) = &element.size {
        drivers.push(format!("size=({},{})", size.x.raw, size.y.raw));
    }
    drivers.push(format!(
        "orientation={:?}{}",
        orientation,
        if element.orientation.is_none() {
            " (default)"
        } else {
            ""
        }
    ));
    out.push(ResolvedTopbarRect {
        path: path.clone(),
        name: element.name.clone(),
        element_type: element.element_type.clone(),
        x: rect.x(),
        y: rect.y(),
        w: rect.w(),
        h: rect.h(),
        orientation,
        drivers: drivers.join("; "),
    });
    for child in &element.children {
        walk_pure(child, rect, &path, options, out);
    }
}

pub fn compose_topbar(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    binding: &TopbarBinding,
) -> Result<TopbarComposeResult> {
    let layout = load_topbar_layout(resolver, registry)?;
    compose_topbar_with_layout(
        resolver,
        registry,
        binding,
        &layout,
        layout.viewport_width,
        layout.viewport_height,
        layout.viewport_width,
        1080,
    )
}

pub fn compose_topbar_at(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    binding: &TopbarBinding,
    width: usize,
    height: usize,
) -> Result<TopbarComposeResult> {
    let layout = load_topbar_layout(resolver, registry)?;
    compose_topbar_with_layout(
        resolver, registry, binding, &layout, width, height, width, height,
    )
}

fn compose_topbar_with_layout(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    binding: &TopbarBinding,
    layout: &TopbarLayout,
    viewport_width: usize,
    viewport_height: usize,
    root_width: usize,
    root_height: usize,
) -> Result<TopbarComposeResult> {
    let localisation = Localisation::load_english(resolver)?;
    let mut options = ComposeOptions::new(viewport_width, viewport_height)
        .with_root_size(root_width, root_height)
        .with_localisation(&localisation);
    apply_binding(&mut options, binding, registry);
    let flag_texture_used = if let Some(flag) = compose_flag_shield(resolver, registry, binding)? {
        options
            .sprite_overrides
            .insert("player_flag".to_string(), flag.image);
        flag.rel_path
    } else {
        None
    };
    let mut compositor = Compositor::new(resolver, registry);
    let mut output = compositor.compose(&layout.root, &options);
    draw_overview_xp_bars(resolver, registry, binding, &mut output)?;
    Ok(TopbarComposeResult {
        layout: layout.clone(),
        output,
        flag_texture_used,
    })
}

pub fn write_topbar_proofs(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    out_dir: &Path,
) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    let sample = compose_topbar(resolver, registry, &TopbarBinding::sample_ire())?;
    let sample_path = out_dir.join("topbar_composited_sample_ire.png");
    write_png(&sample_path, &sample.output.image)?;
    paths.push(sample_path);

    let debug_path = out_dir.join("topbar_debug_rects.png");
    let debug = topbar_debug_overlay(resolver, &sample.output)?;
    write_png(&debug_path, &debug)?;
    paths.push(debug_path);

    let empty = compose_topbar(resolver, registry, &TopbarBinding::default())?;
    let empty_path = out_dir.join("topbar_composited_empty.png");
    write_png(&empty_path, &empty.output.image)?;
    paths.push(empty_path);

    let sheet = text_sample_sheet(resolver)?;
    let sheet_path = out_dir.join("font_sample_sheet.png");
    write_png(&sheet_path, &sheet)?;
    paths.push(sheet_path);
    Ok(paths)
}

pub fn topbar_debug_overlay(resolver: &AssetResolver, output: &ComposeOutput) -> Result<RgbaImage> {
    let mut image = output.image.clone();
    let font = BitmapFont::load(resolver, "hoi_16mbs")?;
    for rect in &output.debug_rects {
        draw_debug_rect(&mut image, rect);
        if rect.w > 2 && rect.h > 2 {
            let label = format!("{}:{}", rect.element_type, rect.name);
            let color = debug_color(rect.depth);
            font.draw_text(
                &mut image,
                rect.x + 2,
                rect.y + 1,
                Some(rect.w.max(24)),
                crate::font::TextAlign::Left,
                &label,
                color,
            );
        }
    }
    Ok(image)
}

fn draw_debug_rect(image: &mut RgbaImage, rect: &DebugRect) {
    let color = debug_color(rect.depth);
    let x0 = rect.x.max(0);
    let y0 = rect.y.max(0);
    let x1 = (rect.x + rect.w - 1).min(image.width as i32 - 1);
    let y1 = (rect.y + rect.h - 1).min(image.height as i32 - 1);
    if x0 > x1 || y0 > y1 {
        return;
    }
    for x in x0..=x1 {
        set_pixel(image, x, y0, color);
        set_pixel(image, x, y1, color);
    }
    for y in y0..=y1 {
        set_pixel(image, x0, y, color);
        set_pixel(image, x1, y, color);
    }
}

fn set_pixel(image: &mut RgbaImage, x: i32, y: i32, rgba: [u8; 4]) {
    if x < 0 || y < 0 || x >= image.width as i32 || y >= image.height as i32 {
        return;
    }
    let offset = (y as usize * image.width + x as usize) * 4;
    image.pixels[offset..offset + 4].copy_from_slice(&rgba);
}

fn debug_color(depth: usize) -> [u8; 4] {
    const COLORS: [[u8; 4]; 6] = [
        [255, 86, 86, 255],
        [86, 210, 255, 255],
        [145, 230, 97, 255],
        [255, 211, 79, 255],
        [211, 124, 255, 255],
        [255, 151, 77, 255],
    ];
    COLORS[depth % COLORS.len()]
}

pub fn binding_fields() -> Vec<BindingField> {
    vec![
        BindingField { field: "country_tag/country_ideology or flag_texture", gui_element: "player_flag", default: "empty shield overlay", notes: "medium flag TGA is masked through GFX_shield_medium" },
        BindingField { field: "political_power", gui_element: "pol_power", default: "0", notes: "" },
        BindingField { field: "manpower", gui_element: "manpower", default: "0", notes: "" },
        BindingField { field: "stability_percent", gui_element: "stability", default: "0%", notes: "" },
        BindingField { field: "war_support_percent", gui_element: "war_support", default: "0%", notes: "" },
        BindingField { field: "command_power", gui_element: "command_power_value", default: "0", notes: "" },
        BindingField { field: "army_experience", gui_element: "experience_value", default: "0", notes: "" },
        BindingField { field: "navy_experience", gui_element: "navy_experience_value", default: "0", notes: "" },
        BindingField { field: "air_experience", gui_element: "air_experience_value", default: "0", notes: "" },
        BindingField { field: "nukes", gui_element: "nukes_value", default: "0", notes: "" },
        BindingField { field: "factory_text or factory_counts", gui_element: "industrial_capacity", default: "0", notes: "factory_text is used when the live topbar exposes a single factory count" },
        BindingField { field: "industrial_ratio", gui_element: "industrial_ratio_bar", default: "0.0", notes: "progress value 0.0..1.0" },
        BindingField { field: "convoys", gui_element: "convoys_count", default: "0", notes: "" },
        BindingField { field: "convoys_ratio", gui_element: "convoys_ratio_bar", default: "0.0", notes: "progress value 0.0..1.0" },
        BindingField { field: "fuel", gui_element: "fuel_value", default: "0d", notes: "" },
        BindingField { field: "fuel_ratio", gui_element: "fuel_ratio_bar", default: "0.0", notes: "progress value 0.0..1.0" },
        BindingField { field: "supply", gui_element: "supply_value", default: "0d", notes: "topbar layout has supply text; no separate strategic resource amount fields are present in topbar.gui" },
        BindingField { field: "supply_ratio", gui_element: "supply_ratio_bar", default: "0.0", notes: "progress value 0.0..1.0" },
        BindingField { field: "world_tension/world_tension_percent", gui_element: "threat_button/threat_icon_glow/threat_value", default: "0%", notes: "selects GFX_world_tension_icon_big_strip and GFX_WT_anim_strip frames from the 0.0..1.0 value" },
        BindingField { field: "date_time", gui_element: "DateText", default: "", notes: "" },
        BindingField { field: "speed", gui_element: "button_speedstep1..5", default: "0", notes: "uses the green frame from GFX_speed_step when present" },
        BindingField { field: "army_xp_ratio/navy_xp_ratio/air_xp_ratio", gui_element: "army_button/navy_button/air_button", default: "0.0", notes: "overlays GFX_topbar_fuel_bar fill anchored to the parsed button rect; vanilla topbar.gui has no separate progressbar child here" },
        BindingField { field: "initiative", gui_element: "initiative_text", default: "", notes: "country-menu alert area" },
        BindingField { field: "decision_timeout_items", gui_element: "decisionview_amount_timeout_items", default: "", notes: "country-menu alert area" },
        BindingField { field: "decision_available_items", gui_element: "decisionview_amount_to_take_items", default: "", notes: "country-menu alert area" },
        BindingField { field: "intel_prepared_or_completed", gui_element: "intelview_ops_prepared_or_completed_items", default: "", notes: "country-menu alert area" },
        BindingField { field: "intel_available", gui_element: "intelview_ops_available_items", default: "", notes: "country-menu alert area" },
        BindingField { field: "prototype_rewards", gui_element: "prototype_rewards_aquired", default: "", notes: "spelling follows vanilla GUI element name" },
        BindingField { field: "non_delivering_contracts", gui_element: "non_delivering_contract_amount", default: "", notes: "international market alert area" },
        BindingField { field: "ongoing_contracts", gui_element: "ongoing_contract_amount", default: "", notes: "international market alert area" },
    ]
}

fn topbar_native_size(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
) -> Result<(usize, usize)> {
    let sprite = registry
        .get("GFX_topbar_background_extended")
        .ok_or_else(|| GameUiError::missing("GFX_topbar_background_extended missing"))?;
    let decoded = resolver.decode_sprite(sprite)?;
    let frame = decoded
        .frames
        .first()
        .ok_or_else(|| GameUiError::missing("topbar background has no frame"))?;
    Ok((frame.width, frame.height))
}

fn apply_binding(options: &mut ComposeOptions, binding: &TopbarBinding, registry: &SpriteRegistry) {
    let text = &mut options.text_overrides;
    text.insert(
        "pol_power".to_string(),
        binding
            .political_power
            .clone()
            .unwrap_or_else(|| "0".to_string()),
    );
    text.insert(
        "manpower".to_string(),
        binding.manpower.clone().unwrap_or_else(|| "0".to_string()),
    );
    text.insert(
        "stability".to_string(),
        binding
            .stability_percent
            .clone()
            .unwrap_or_else(|| "0%".to_string()),
    );
    text.insert(
        "war_support".to_string(),
        binding
            .war_support_percent
            .clone()
            .unwrap_or_else(|| "0%".to_string()),
    );
    text.insert(
        "command_power_value".to_string(),
        binding
            .command_power
            .clone()
            .unwrap_or_else(|| "0".to_string()),
    );
    text.insert(
        "experience_value".to_string(),
        binding
            .army_experience
            .clone()
            .unwrap_or_else(|| "0".to_string()),
    );
    text.insert(
        "navy_experience_value".to_string(),
        binding
            .navy_experience
            .clone()
            .unwrap_or_else(|| "0".to_string()),
    );
    text.insert(
        "air_experience_value".to_string(),
        binding
            .air_experience
            .clone()
            .unwrap_or_else(|| "0".to_string()),
    );
    text.insert(
        "nukes_value".to_string(),
        binding.nukes.clone().unwrap_or_else(|| "0".to_string()),
    );
    text.insert(
        "industrial_capacity".to_string(),
        binding.factory_text.clone().unwrap_or_else(|| {
            binding
                .factory_counts
                .as_ref()
                .map(FactoryCounts::to_topbar_text)
                .unwrap_or_else(|| "0".to_string())
        }),
    );
    text.insert(
        "convoys_count".to_string(),
        binding.convoys.clone().unwrap_or_else(|| "0".to_string()),
    );
    text.insert(
        "fuel_value".to_string(),
        binding.fuel.clone().unwrap_or_else(|| "0d".to_string()),
    );
    text.insert(
        "supply_value".to_string(),
        binding.supply.clone().unwrap_or_else(|| "0d".to_string()),
    );
    text.insert(
        "threat_value".to_string(),
        binding
            .world_tension_percent
            .clone()
            .or_else(|| binding.world_tension.map(format_percent))
            .unwrap_or_else(|| "0%".to_string()),
    );
    text.insert(
        "DateText".to_string(),
        binding.date_time.clone().unwrap_or_default(),
    );
    text.insert(
        "initiative_text".to_string(),
        binding.initiative.clone().unwrap_or_default(),
    );
    text.insert(
        "decisionview_amount_timeout_items".to_string(),
        binding.decision_timeout_items.clone().unwrap_or_default(),
    );
    text.insert(
        "decisionview_amount_to_take_items".to_string(),
        binding.decision_available_items.clone().unwrap_or_default(),
    );
    text.insert(
        "intelview_ops_prepared_or_completed_items".to_string(),
        binding
            .intel_prepared_or_completed
            .clone()
            .unwrap_or_default(),
    );
    text.insert(
        "intelview_ops_available_items".to_string(),
        binding.intel_available.clone().unwrap_or_default(),
    );
    text.insert(
        "prototype_rewards_aquired".to_string(),
        binding.prototype_rewards.clone().unwrap_or_default(),
    );
    text.insert(
        "non_delivering_contract_amount".to_string(),
        binding.non_delivering_contracts.clone().unwrap_or_default(),
    );
    text.insert(
        "ongoing_contract_amount".to_string(),
        binding.ongoing_contracts.clone().unwrap_or_default(),
    );

    options.progress_overrides.insert(
        "industrial_ratio_bar".to_string(),
        binding.industrial_ratio.unwrap_or(0.0),
    );
    options.progress_overrides.insert(
        "convoys_ratio_bar".to_string(),
        binding.convoys_ratio.unwrap_or(0.0),
    );
    options.progress_overrides.insert(
        "fuel_ratio_bar".to_string(),
        binding.fuel_ratio.unwrap_or(0.0),
    );
    options.progress_overrides.insert(
        "supply_ratio_bar".to_string(),
        binding.supply_ratio.unwrap_or(0.0),
    );

    let speed = binding.speed.unwrap_or(0).min(5);
    let speed_green_frame = registry
        .get("GFX_speed_step")
        .map(|sprite| if sprite.no_of_frames > 1 { 1 } else { 0 })
        .unwrap_or(1);
    for step in 1..=5 {
        options.frame_overrides.insert(
            format!("button_speedstep{step}"),
            if step <= speed { speed_green_frame } else { 0 },
        );
    }

    let tension = binding.world_tension.unwrap_or_else(|| {
        binding
            .world_tension_percent
            .as_deref()
            .and_then(parse_percent)
            .unwrap_or(0.0)
    });
    if let Some(sprite) = registry.get("GFX_world_tension_icon_big_strip") {
        options.frame_overrides.insert(
            "threat_button".to_string(),
            frame_from_fraction(tension, sprite.no_of_frames as usize),
        );
    }
    if let Some(sprite) = registry.get("GFX_WT_anim_strip") {
        options.frame_overrides.insert(
            "threat_icon_glow".to_string(),
            frame_from_fraction(tension, sprite.no_of_frames as usize),
        );
    }
}

#[derive(Clone, Debug)]
struct FlagShield {
    image: RgbaImage,
    rel_path: Option<String>,
}

fn compose_flag_shield(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    binding: &TopbarBinding,
) -> Result<Option<FlagShield>> {
    let Some(flag) = resolve_flag(resolver, binding)? else {
        return Ok(None);
    };
    let shield = registry
        .get("GFX_shield_medium")
        .ok_or_else(|| GameUiError::missing("GFX_shield_medium missing"))?;
    let overlay_rel = shield
        .texture_files
        .first()
        .ok_or_else(|| GameUiError::missing("GFX_shield_medium overlay texture missing"))?
        .path
        .clone();
    let mask_rel = shield
        .texture_files
        .get(1)
        .ok_or_else(|| GameUiError::missing("GFX_shield_medium mask texture missing"))?
        .path
        .clone();
    let overlay = resolver.decode_texture(&overlay_rel)?.image;
    let mask = resolver.decode_texture(&mask_rel)?.image;
    let mut image = blank_image(overlay.width, overlay.height);
    draw_scaled(&mut image, &flag.image, 0, 0, overlay.width, overlay.height);
    apply_mask(&mut image, &mask);
    draw_image(&mut image, &overlay, 0, 0);
    Ok(Some(FlagShield {
        image,
        rel_path: Some(flag.rel_path),
    }))
}

#[derive(Clone, Debug)]
struct ResolvedFlag {
    rel_path: String,
    image: RgbaImage,
}

/// Ordered flag-texture candidate paths for a country, encoding the engine's
/// flag-name precedence. The engine identifies a flag by a *flag name* — the
/// `set_cosmetic_tag` cosmetic tag when present, else the country tag — suffixed
/// with the current ideology, falling back to the base (un-suffixed) flag of the
/// same name. FILE-grounded from the shipped `gfx/flags` inventory:
///   * `GER.tga` (base) is ABSENT — only `GER_<ideology>.tga` ship, so the
///     ideology variant must precede the base;
///   * cosmetic `RAJ_UK.tga` ships but `RAJ_UK_neutrality.tga` does NOT, so a
///     cosmetic's base flag must be tried before the real tag's flags.
/// Therefore all candidates for the cosmetic name precede all candidates for the
/// real tag. `gfx/flags` (full-res), `medium/`, and `small/` ship as parallel
/// per-name sets; the size folder is the render context's (residual: the exact
/// size-folder selection law is UNTRACED — see UI_LAYOUT_LAW.md).
pub fn flag_candidates(
    tag: Option<&str>,
    ideology: Option<&str>,
    cosmetic_tag: Option<&str>,
    explicit: Option<&str>,
) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(explicit) = explicit {
        out.push(explicit.to_string());
    }
    let mut names: Vec<String> = Vec::new();
    // Cosmetic tags are case-sensitive identifiers (e.g. `RAJ_UK`); the base
    // tag is upper-cased.
    if let Some(cosmetic) = cosmetic_tag.filter(|value| !value.is_empty()) {
        names.push(cosmetic.to_string());
    }
    if let Some(tag) = tag {
        names.push(tag.to_ascii_uppercase());
    }
    for name in names {
        for dir in ["gfx/flags", "gfx/flags/medium", "gfx/flags/small"] {
            if let Some(ideology) = ideology {
                out.push(format!("{dir}/{name}_{ideology}.tga"));
            }
            out.push(format!("{dir}/{name}.tga"));
        }
    }
    out
}

fn resolve_flag(resolver: &AssetResolver, binding: &TopbarBinding) -> Result<Option<ResolvedFlag>> {
    let candidates = flag_candidates(
        binding.country_tag.as_deref(),
        binding.country_ideology.as_deref(),
        binding.cosmetic_tag.as_deref(),
        binding.flag_texture.as_deref(),
    );
    for candidate in candidates {
        if resolver.resolve_optional_file(&candidate)?.is_some() {
            let decoded = resolver.decode_texture(&candidate)?;
            return Ok(Some(ResolvedFlag {
                rel_path: decoded.rel_path,
                image: decoded.image,
            }));
        }
    }
    Ok(None)
}

fn apply_mask(image: &mut RgbaImage, mask: &RgbaImage) {
    if image.width == 0 || image.height == 0 || mask.width == 0 || mask.height == 0 {
        return;
    }
    for y in 0..image.height {
        for x in 0..image.width {
            let mx = x * mask.width / image.width;
            let my = y * mask.height / image.height;
            let mask_offset = (my * mask.width + mx) * 4;
            let coverage = mask.pixels[mask_offset + 3].max(
                mask.pixels[mask_offset]
                    .max(mask.pixels[mask_offset + 1])
                    .max(mask.pixels[mask_offset + 2]),
            );
            let offset = (y * image.width + x) * 4 + 3;
            image.pixels[offset] =
                ((u16::from(image.pixels[offset]) * u16::from(coverage) + 127) / 255) as u8;
        }
    }
}

pub fn text_sample_sheet(resolver: &AssetResolver) -> Result<RgbaImage> {
    let fonts = [
        BitmapFont::load(resolver, "hoi_16mbs")?,
        BitmapFont::load(resolver, "hoi_18mbs")?,
        BitmapFont::load(resolver, "hoi_26mbs")?,
    ];
    let mut image = blank_image(1720, 700);
    let mut y = 10;
    let samples = [
        "ASCII 32-126:  !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~",
        "Date: 9 Nov 1943, 17:00",
        "Digits: 0% 1% 5% 9% 10% 51% 78% 100% 12/24/6 216 91d",
        "Color escapes stripped: §Yyellow§! §Rred§! §Ggreen§!",
    ];
    let colors = [
        [221, 214, 190, 255],
        [255, 255, 255, 255],
        [134, 210, 120, 255],
    ];
    for font in &fonts {
        for dark_background in [false, true] {
            let row_step = font.line_height.max(16) + 4;
            let block_height = samples[..3].len() as i32 * row_step + 10;
            if dark_background {
                let image_width = image.width as i32;
                fill_rect(
                    &mut image,
                    0,
                    y - 5,
                    image_width,
                    block_height,
                    [18, 21, 24, 255],
                );
            }
            for (idx, sample) in samples[..3].iter().enumerate() {
                let sample = format!("{} {}", font.name, sample);
                font.draw_text(
                    &mut image,
                    10,
                    y,
                    Some(1700),
                    crate::font::TextAlign::Left,
                    &sample,
                    colors[idx % colors.len()],
                );
                y += row_step;
            }
            y += 10;
        }
        y += 8;
    }
    Ok(image)
}

fn fill_rect(image: &mut RgbaImage, x: i32, y: i32, w: i32, h: i32, rgba: [u8; 4]) {
    for dy in y.max(0)..(y + h).min(image.height as i32) {
        for dx in x.max(0)..(x + w).min(image.width as i32) {
            let offset = (dy as usize * image.width + dx as usize) * 4;
            image.pixels[offset..offset + 4].copy_from_slice(&rgba);
        }
    }
}

pub fn topbar_structure_summary(layout: &TopbarLayout) -> BTreeMap<String, usize> {
    fn walk(element: &GuiElement, counts: &mut BTreeMap<String, usize>) {
        *counts.entry(element.element_type.clone()).or_insert(0) += 1;
        for child in &element.children {
            walk(child, counts);
        }
    }
    let mut counts = BTreeMap::new();
    walk(&layout.root, &mut counts);
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::AssetResolver;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn flag_variant_prefers_full_size_ideology_before_medium() {
        let root = temp_root("flag_order");
        write_tga(&root.join("gfx/flags/IRE_neutrality.tga"), [0, 255, 0, 255]);
        write_tga(
            &root.join("gfx/flags/medium/IRE_neutrality.tga"),
            [0, 0, 255, 255],
        );
        let resolver = AssetResolver::new(vec![root.clone()]);
        let binding = TopbarBinding {
            country_tag: Some("IRE".to_string()),
            country_ideology: Some("neutrality".to_string()),
            ..TopbarBinding::default()
        };

        let flag = resolve_flag(&resolver, &binding).unwrap().unwrap();
        assert_eq!(flag.rel_path, "gfx/flags/IRE_neutrality.tga");
        assert_eq!(flag.image.pixels[0..3], [0, 255, 0]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn flag_candidate_order_puts_cosmetic_before_tag() {
        let cands = flag_candidates(Some("raj"), Some("neutrality"), Some("RAJ_UK"), None);
        // All cosmetic-name candidates precede any real-tag candidate.
        let first_tag = cands.iter().position(|c| !c.contains("RAJ_UK")).unwrap();
        let last_cosmetic = cands.iter().rposition(|c| c.contains("RAJ_UK")).unwrap();
        assert!(
            last_cosmetic < first_tag,
            "cosmetic must precede tag: {cands:?}"
        );
        // Within a name, the ideology variant precedes the base.
        assert_eq!(cands[0], "gfx/flags/RAJ_UK_neutrality.tga");
        assert_eq!(cands[1], "gfx/flags/RAJ_UK.tga");
    }

    #[test]
    fn cosmetic_base_flag_beats_real_tag_flag() {
        let root = temp_root("flag_cosmetic");
        // The cosmetic's ideology variant is ABSENT; its base is present. The
        // real tag's ideology flag also exists but must NOT win.
        write_tga(&root.join("gfx/flags/RAJ_UK.tga"), [10, 20, 30, 255]);
        write_tga(&root.join("gfx/flags/RAJ_neutrality.tga"), [200, 0, 0, 255]);
        let resolver = AssetResolver::new(vec![root.clone()]);
        let binding = TopbarBinding {
            country_tag: Some("RAJ".to_string()),
            country_ideology: Some("neutrality".to_string()),
            cosmetic_tag: Some("RAJ_UK".to_string()),
            ..TopbarBinding::default()
        };
        let flag = resolve_flag(&resolver, &binding).unwrap().unwrap();
        assert_eq!(flag.rel_path, "gfx/flags/RAJ_UK.tga");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn frame_selection_maps_fraction_to_existing_frames() {
        assert_eq!(frame_from_fraction(0.0, 10), 0);
        assert_eq!(frame_from_fraction(0.51, 10), 5);
        assert_eq!(frame_from_fraction(1.0, 10), 9);
        assert_eq!(frame_from_fraction(1.0, 33), 32);
        assert_eq!(frame_from_fraction(-1.0, 10), 0);
    }

    #[test]
    fn formats_topbar_date_like_hoi4() {
        assert_eq!(format_topbar_datetime(2, 1, 7, 1944), "2:00, 1 Jul, 1944");
    }

    #[test]
    fn formats_scaled_values_with_two_decimals() {
        assert_eq!(format_scaled_short(66_100), "66.10K");
        assert_eq!(format_scaled_short(2_500_000), "2.50M");
        assert_eq!(format_percent(0.735), "74%");
    }

    fn hoi4_root() -> Option<PathBuf> {
        let root = std::env::var_os("BATTLE_PLAN_GAME_ROOT").or_else(|| std::env::var_os("HOI4_GAME_ROOT"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(crate::probe::DEFAULT_GAME_ROOT));
        if root.is_dir() {
            Some(root)
        } else {
            None
        }
    }

    fn find<'a>(rects: &'a [ResolvedTopbarRect], name: &str) -> &'a ResolvedTopbarRect {
        rects
            .iter()
            .find(|r| r.name == name)
            .unwrap_or_else(|| panic!("topbar element `{name}` not resolved"))
    }

    /// GATE: the topbar element rects resolved purely from `interface/topbar.gui`
    /// must follow the decompiled engine layout law
    /// (origin(child) = origin(parent) + anchor_fraction(orientation)·size(parent)
    /// + position). Expected values are hand-derived from the exact `.gui` fields
    /// at a 1920x1080 viewport and reported alongside their drivers.
    #[test]
    fn topbar_rects_follow_engine_layout_law() {
        let Some(root) = hoi4_root() else {
            eprintln!("SKIP topbar gate: the game root missing");
            return;
        };
        let resolver = AssetResolver::new(vec![root]);
        // Load the layout directly from topbar.gui (no sprite/texture decode is
        // needed for a pure-law rect resolution).
        let resolved = resolver
            .resolve_file(TOPBAR_GUI)
            .expect("resolve topbar.gui");
        let doc = parse_file(&resolved.path).expect("parse topbar.gui");
        let mut layouts = LayoutSet::default();
        layouts.add_document(&resolved.rel_path, &doc);
        let root_el = layouts
            .documents
            .iter()
            .flat_map(|d| d.roots.iter())
            .find(|r| {
                r.name.eq_ignore_ascii_case(TOPBAR_WINDOW_NAME)
                    && r.element_type.eq_ignore_ascii_case("containerWindowType")
            })
            .cloned()
            .expect("topbar containerWindowType");
        let layout = TopbarLayout {
            source_file: resolved.rel_path,
            window_name: TOPBAR_WINDOW_NAME.to_string(),
            root: root_el,
            viewport_width: 1920,
            viewport_height: 1080,
        };
        let rects = resolve_topbar_rects_pure(&layout, 1920, 1080);

        // (name, x, y, w, h) — each derived from the .gui fields + the law.
        // root: position=(-4,-1) size=(100%%,100%%) UPPER_LEFT(default)
        //   -> anchor (0,0) of viewport, size = full viewport, +position.
        let expected = [
            ("topbar", -4, -1, 1920, 1080),
            // player_flag: position=(12,14) UPPER_LEFT under root(-4,-1)
            ("player_flag", 8, 13, 0, 0),
            // land: position=(796,5) size=(56,28) UPPER_LEFT under root
            ("land", 792, 4, 56, 28),
            // experience_value: position=(23,5) UPPER_LEFT(default) under land(792,4)
            ("experience_value", 815, 9, 0, 0),
            // menu_button: position=(-35,7) UPPER_RIGHT -> anchor (1920,0) of root
            ("menu_button", 1881, 6, 0, 0),
            // threat_button: position=(-96,20) UPPER_RIGHT under root
            ("threat_button", 1820, 19, 0, 0),
            // time_control_window: position=(-378,7) size=(253,30) UPPER_RIGHT
            ("time_control_window", 1538, 6, 253, 30),
            // DateText: position=(-175,4) UPPER_RIGHT under time_control_window(1538,6,253,30)
            //   -> anchor (253,0) => 1538+253-175 = 1616, 6+4 = 10
            ("DateText", 1616, 10, 0, 0),
        ];

        for (name, x, y, w, h) in expected {
            let r = find(&rects, name);
            println!(
                "{:<22} rect=({},{},{},{})  [{}]",
                r.path, r.x, r.y, r.w, r.h, r.drivers
            );
            assert_eq!(
                (r.x, r.y, r.w, r.h),
                (x, y, w, h),
                "rect mismatch for {name}"
            );
        }

        // Nested right-anchoring must compose parent→child, not re-anchor to the
        // viewport: DateText sits inside the right-anchored time_control_window.
        let tcw = find(&rects, "time_control_window");
        let date = find(&rects, "DateText");
        assert!(date.x > tcw.x && date.x < tcw.x + tcw.w);
        assert_eq!(date.orientation, Orientation::UpperRight);
    }

    fn temp_root(label: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        path.push(format!("game_ui_topbar_{label}_{nonce}"));
        path
    }

    fn write_tga(path: &Path, rgba: [u8; 4]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut bytes = vec![0u8; 18];
        bytes[2] = 2;
        bytes[12] = 1;
        bytes[14] = 1;
        bytes[16] = 32;
        bytes[17] = 0x20;
        bytes.extend_from_slice(&[rgba[2], rgba[1], rgba[0], rgba[3]]);
        fs::write(path, bytes).unwrap();
    }
}
