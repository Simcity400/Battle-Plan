
use crate::*;
use replay_core::{Leaf, SessionMetadataView, ViewStatus};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;

pub const MAPICONS_GUI_SOURCE: &str = "interface/mapicons.gui";
pub const MAPICONS_GFX_SOURCE: &str = "interface/mapicons.gfx";
pub const CORE_GFX_SOURCE: &str = "interface/core.gfx";

pub const VP_STRIP_SPRITE_TEXTURE: &str = "gfx/interface/onmap_victorypoints_strip.dds";
pub const VP_STRIP_FRAME_COUNT: u32 = 20;
pub const VP_STRIP_SPRITE_NAME: &str = "GFX_onmap_victorypoints_strip";

pub const HOI_20BS_FNT_SOURCE: &str = "gfx/fonts/hoi_20bs.fnt";
pub const HOI_20BS_ATLAS_SOURCE: &str = "gfx/fonts/hoi_20bs.dds";
pub const VP_LABEL_FONT_NAME: &str = "hoi_20bs";


pub const MAPICON_CONTAINER_WIDTH: i32 = 160;
pub const MAPICON_CONTAINER_HEIGHT: i32 = 30;
pub const MAPICON_ICON_OFFSET_X: i32 = -5;
pub const MAPICON_ICON_OFFSET_Y: i32 = 0;
pub const MAPICON_TEXT_OFFSET_X: i32 = 26;
pub const MAPICON_TEXT_OFFSET_Y: i32 = 5;
pub const MAPICON_TEXT_MAX_WIDTH: i32 = 149;
pub const MAPICON_TEXT_MAX_HEIGHT: i32 = 30;

pub const VICTORY_POINTS_LOC_PREFIX: &str = "VICTORY_POINTS_";


pub const VICTORY_POINT_LEVELS_KEY: &str = "VICTORY_POINT_LEVELS";
pub const VICTORY_POINT_MAP_ICON_AFTER_KEY: &str = "VICTORY_POINT_MAP_ICON_AFTER";
pub const VICTORY_POINT_MAP_ICON_TEXT_CUTOFF_KEY: &str = "VICTORY_POINT_MAP_ICON_TEXT_CUTOFF";
pub const VICTORY_POINT_MAP_ICON_TEXT_CUTOFF_MIN_KEY: &str =
    "VICTORY_POINT_MAP_ICON_TEXT_CUTOFF_MIN";
pub const VICTORY_POINT_MAP_ICON_TEXT_CUTOFF_MAX_KEY: &str =
    "VICTORY_POINT_MAP_ICON_TEXT_CUTOFF_MAX";
pub const VICTORY_POINT_MAP_ICON_DOT_CUTOFF_MIN_KEY: &str = "VICTORY_POINT_MAP_ICON_DOT_CUTOFF_MIN";
pub const VICTORY_POINT_MAP_ICON_DOT_CUTOFF_MAX_KEY: &str = "VICTORY_POINT_MAP_ICON_DOT_CUTOFF_MAX";
pub const VICTORY_POINT_MAP_ICON_MAX_VICTORY_POINTS_FOR_PERCENT_KEY: &str =
    "VICTORY_POINT_MAP_ICON_MAX_VICTORY_POINTS_FOR_PERCENT";
pub const VICTORY_POINT_MAP_ICON_CAPITAL_CUTOFF_MAX_KEY: &str =
    "VICTORY_POINT_MAP_ICON_CAPITAL_CUTOFF_MAX";
pub const CAPITAL_ICON_CUTOFF_KEY: &str = "CAPITAL_ICON_CUTOFF";

#[derive(Clone, Debug, PartialEq)]
pub struct VpIconDefines {
    pub levels: u32,
    pub map_icon_after: Vec<i64>,
    pub text_cutoff: Vec<f32>,
    pub text_cutoff_min: f32,
    pub text_cutoff_max: f32,
    pub dot_cutoff_min: f32,
    pub dot_cutoff_max: f32,
    pub max_victory_points_for_percent: f32,
    pub capital_cutoff_max: f32,
    pub capital_icon_cutoff: f32,
}

impl VpIconDefines {
    pub fn validate(&self) -> Result<()> {
        if self.levels == 0
            || self.map_icon_after.len() < self.levels as usize
            || self.text_cutoff.len() < self.levels as usize
            || !self.text_cutoff.iter().all(|value| value.is_finite())
            || !self.text_cutoff_min.is_finite()
            || !self.text_cutoff_max.is_finite()
            || self.text_cutoff_max < self.text_cutoff_min
            || !self.dot_cutoff_min.is_finite()
            || !self.dot_cutoff_max.is_finite()
            || self.dot_cutoff_max <= self.dot_cutoff_min
            || !self.max_victory_points_for_percent.is_finite()
            || self.max_victory_points_for_percent <= 0.0
            || !self.capital_cutoff_max.is_finite()
            || !self.capital_icon_cutoff.is_finite()
            || self.capital_icon_cutoff <= 0.0
        {
            return Err("VP/capital icon graphics defines are invalid".into());
        }
        Ok(())
    }

    pub fn tier_for_vp(&self, vp_value: i32) -> u32 {
        let mut tier = 0u32;
        for (idx, threshold) in self.map_icon_after.iter().enumerate() {
            if i64::from(vp_value) >= *threshold {
                tier = idx as u32;
            }
        }
        tier.min(self.levels.saturating_sub(1))
    }

    pub fn dot_visible(&self, kind: FurnitureMarkerKind, vp_value: i32, cam_dist: f32) -> bool {
        match kind {
            FurnitureMarkerKind::Capital => cam_dist < self.capital_icon_cutoff,
            FurnitureMarkerKind::VictoryPoint => {
                let percent = vp_value as f32 / self.max_victory_points_for_percent;
                let cutoff =
                    self.dot_cutoff_min + (self.dot_cutoff_max - self.dot_cutoff_min) * percent;
                cam_dist < self.dot_cutoff_max && cam_dist < cutoff
            }
        }
    }

    pub fn text_visible(
        &self,
        kind: FurnitureMarkerKind,
        vp_value: i32,
        cam_dist: f32,
        resources_mode: bool,
    ) -> bool {
        if resources_mode {
            return false;
        }
        match kind {
            FurnitureMarkerKind::Capital => cam_dist < self.capital_cutoff_max,
            FurnitureMarkerKind::VictoryPoint => {
                let percent = vp_value as f32 / self.max_victory_points_for_percent;
                let cutoff =
                    self.text_cutoff_min + (self.text_cutoff_max - self.text_cutoff_min) * percent;
                cam_dist < cutoff
            }
        }
    }
}


pub const VP_STRIP_COLOR_SLOTS: u32 = 5;
pub const VP_STRIP_COLOR_ENEMY_RED: u32 = 0;
pub const VP_STRIP_COLOR_ENGINE_UNREACHED_1: u32 = 1;
pub const VP_STRIP_COLOR_SELF_OR_FACTION: u32 = 2;
pub const VP_STRIP_COLOR_ENGINE_UNREACHED_3: u32 = 3;
pub const VP_STRIP_COLOR_NEUTRAL_GRAY: u32 = 4;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VpIconDiplomaticPairState {
    pub same_faction: bool,
    pub at_war: bool,
}

pub fn vp_icon_relation_column(
    owner_tag: Option<&str>,
    viewed_tag: Option<&str>,
    pair_state: VpIconDiplomaticPairState,
) -> u32 {
    let Some(owner_tag) = owner_tag.filter(|tag| !tag.is_empty()) else {
        return VP_STRIP_COLOR_NEUTRAL_GRAY;
    };
    let Some(viewed_tag) = viewed_tag.filter(|tag| !tag.is_empty()) else {
        return VP_STRIP_COLOR_NEUTRAL_GRAY;
    };
    if owner_tag == viewed_tag || pair_state.same_faction {
        return VP_STRIP_COLOR_SELF_OR_FACTION;
    }
    if pair_state.at_war {
        return VP_STRIP_COLOR_ENEMY_RED;
    }
    VP_STRIP_COLOR_NEUTRAL_GRAY
}

pub fn vp_strip_frame_index(kind: FurnitureMarkerKind, dot_tier: u32, color_slot: u32) -> u32 {
    let shape_base = match kind {
        FurnitureMarkerKind::Capital => 0,
        FurnitureMarkerKind::VictoryPoint => match dot_tier.min(2) {
            2 => 5,
            1 => 10,
            _ => 15,
        },
    };
    shape_base + color_slot.min(VP_STRIP_COLOR_SLOTS - 1)
}

pub fn load_vp_strip_rgba(resolver: &GameFileResolver) -> Result<RgbaFontAtlas> {
    let asset = crate::map_icon_runtime::load_vp_strip_asset(resolver)?;
    Ok(RgbaFontAtlas {
        width: asset.image.width,
        height: asset.image.height,
        pixels: asset.image.pixels,
    })
}

pub struct FurnitureStaticData {
    pub state_victory_points: BTreeMap<u32, BTreeMap<u32, i32>>,
    pub capital_states: BTreeMap<String, u32>,
    pub localisation: BTreeMap<String, String>,
    pub vp_icon_defines: VpIconDefines,
}

pub struct HeightmapGrid {
    pub width: usize,
    pub height: usize,
    raw: Vec<u8>,
}

impl HeightmapGrid {
    pub fn world_y(&self, map_x: f32, map_y: f32) -> f32 {
        if self.width == 0 || self.height == 0 {
            return 0.0;
        }
        let x = (map_x.max(0.0) as usize).min(self.width - 1);
        let y = (map_y.max(0.0) as usize).min(self.height - 1);
        f32::from(self.raw[y * self.width + x]) * 0.1
    }
}

pub fn heightmap_grid_from_resolver(resolver: &GameFileResolver) -> Result<HeightmapGrid> {
    let path = resolver.resolve_required("map/heightmap.bmp", "heightmap.bmp")?;
    let image = load_bmp_indexed(&path)?;
    Ok(HeightmapGrid {
        width: image.width,
        height: image.height,
        raw: image.pixels,
    })
}

pub fn furniture_static_data_from_resolver(
    resolver: &GameFileResolver,
    language: &str,
) -> Result<FurnitureStaticData> {
    let state_files = resolver.resolve_dir_files("history/states", Some("txt"))?;
    let (states, _province_to_state, _state_owners) = load_states(&state_files)?;
    let mut state_victory_points = BTreeMap::new();
    for (state_id, meta) in &states {
        if !meta.victory_points.is_empty() {
            state_victory_points.insert(*state_id, meta.victory_points.clone());
        }
    }
    Ok(FurnitureStaticData {
        state_victory_points,
        capital_states: capital_states_from_resolver(resolver)?,
        localisation: localisation_table_from_resolver(resolver, language)?,
        vp_icon_defines: vp_icon_defines_from_resolver(resolver)?,
    })
}

pub fn vp_icon_defines_from_resolver(resolver: &GameFileResolver) -> Result<VpIconDefines> {
    let path = resolver.resolve_required(BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE, "graphics defines lua")?;
    let source = latin1_bytes_to_string(&std::fs::read(&path)?);
    parse_vp_icon_defines(&source)
}

pub fn parse_vp_icon_defines(source: &str) -> Result<VpIconDefines> {
    let scalar = |key: &str| -> Result<f32> { parse_lua_number_define(source, key) };
    let list_i = |key: &str| -> Result<Vec<i64>> {
        parse_lua_number_list(source, key)
            .map(|list| list.into_iter().map(|value| value as i64).collect())
            .ok_or_else(|| format!("{BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE} missing list {key}").into())
    };
    let list_f = |key: &str| -> Result<Vec<f32>> {
        parse_lua_number_list(source, key)
            .ok_or_else(|| format!("{BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE} missing list {key}").into())
    };
    let defines = VpIconDefines {
        levels: scalar(VICTORY_POINT_LEVELS_KEY)? as u32,
        map_icon_after: list_i(VICTORY_POINT_MAP_ICON_AFTER_KEY)?,
        text_cutoff: list_f(VICTORY_POINT_MAP_ICON_TEXT_CUTOFF_KEY)?,
        text_cutoff_min: scalar(VICTORY_POINT_MAP_ICON_TEXT_CUTOFF_MIN_KEY)?,
        text_cutoff_max: scalar(VICTORY_POINT_MAP_ICON_TEXT_CUTOFF_MAX_KEY)?,
        dot_cutoff_min: scalar(VICTORY_POINT_MAP_ICON_DOT_CUTOFF_MIN_KEY)?,
        dot_cutoff_max: scalar(VICTORY_POINT_MAP_ICON_DOT_CUTOFF_MAX_KEY)?,
        max_victory_points_for_percent: scalar(
            VICTORY_POINT_MAP_ICON_MAX_VICTORY_POINTS_FOR_PERCENT_KEY,
        )?,
        capital_cutoff_max: scalar(VICTORY_POINT_MAP_ICON_CAPITAL_CUTOFF_MAX_KEY)?,
        capital_icon_cutoff: scalar(CAPITAL_ICON_CUTOFF_KEY)?,
    };
    defines.validate()?;
    Ok(defines)
}

pub(crate) fn parse_lua_number_list(source: &str, key: &str) -> Option<Vec<f32>> {
    for line in source.lines() {
        let code = line.split("--").next().unwrap_or(line);
        let Some(idx) = find_assignment(code, key, 0) else {
            continue;
        };
        let rest = code[idx..].trim_start();
        let Some(open) = rest.strip_prefix('{') else {
            continue;
        };
        let Some(end) = open.find('}') else {
            continue;
        };
        let values = open[..end]
            .split(',')
            .filter_map(|token| token.trim().parse::<f32>().ok())
            .collect::<Vec<_>>();
        if values.is_empty() {
            continue;
        }
        return Some(values);
    }
    None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FurnitureMarkerKind {
    VictoryPoint,
    Capital,
}

impl FurnitureMarkerKind {
    pub fn as_str(self) -> &'static str {
        match self {
            FurnitureMarkerKind::VictoryPoint => "victory_point",
            FurnitureMarkerKind::Capital => "capital",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FurnitureMarker {
    pub province_id: u32,
    pub state_id: u32,
    pub owner: String,
    pub anchor_x: f32,
    pub anchor_y: f32,
    pub vp_value: i32,
    pub label: String,
    pub label_resolved: bool,
    pub kind: FurnitureMarkerKind,
    pub dot_tier: u32,
    pub pixel_count: u64,
}

pub struct FurnitureLawInputs<'a> {
    pub province_ids: &'a [u32],
    pub width: usize,
    pub height: usize,
    pub province_to_state: &'a BTreeMap<u32, u32>,
    pub state_owners: &'a BTreeMap<u32, String>,
    pub state_victory_points: &'a BTreeMap<u32, BTreeMap<u32, i32>>,
    pub capital_states: &'a BTreeMap<String, u32>,
    pub localisation: &'a BTreeMap<String, String>,
    pub vp_icon_defines: Option<&'a VpIconDefines>,
}

pub fn compute_furniture_markers(inputs: &FurnitureLawInputs) -> Vec<FurnitureMarker> {
    let expected = inputs.width.saturating_mul(inputs.height);
    if inputs.province_ids.len() != expected || expected == 0 {
        return Vec::new();
    }

    let mut vp_province_state: HashMap<u32, (u32, i32)> = HashMap::new();
    for (state_id, vps) in inputs.state_victory_points {
        for (province_id, value) in vps {
            vp_province_state.insert(*province_id, (*state_id, *value));
        }
    }
    if vp_province_state.is_empty() {
        return Vec::new();
    }

    let mut centroids: HashMap<u32, (f64, f64, u64)> = HashMap::new();
    for y in 0..inputs.height {
        let row = y * inputs.width;
        for x in 0..inputs.width {
            let province = inputs.province_ids[row + x];
            if province == 0 {
                continue;
            }
            if vp_province_state.contains_key(&province) {
                let entry = centroids.entry(province).or_insert((0.0, 0.0, 0));
                entry.0 += x as f64;
                entry.1 += y as f64;
                entry.2 += 1;
            }
        }
    }

    let mut capital_choice: HashMap<u32, (i32, u32)> = HashMap::new();
    for (province_id, (state_id, vp_value)) in &vp_province_state {
        let entry = capital_choice
            .entry(*state_id)
            .or_insert((*vp_value, *province_id));
        if *vp_value > entry.0 || (*vp_value == entry.0 && *province_id < entry.1) {
            *entry = (*vp_value, *province_id);
        }
    }
    let mut markers = Vec::new();
    for (province_id, (state_id, vp_value)) in &vp_province_state {
        let Some((sum_x, sum_y, count)) = centroids.get(province_id).copied() else {
            continue;
        };
        if count == 0 {
            continue;
        }
        let owner = inputs
            .state_owners
            .get(state_id)
            .cloned()
            .unwrap_or_default();
        let is_capital = (!owner.is_empty())
            .then(|| inputs.capital_states.get(&owner).copied())
            .flatten()
            .map(|capital_state| {
                capital_state == *state_id
                    && capital_choice
                        .get(state_id)
                        .map(|(_, chosen)| chosen == province_id)
                        .unwrap_or(false)
            })
            .unwrap_or(false);
        let loc_key = format!("{VICTORY_POINTS_LOC_PREFIX}{province_id}");
        let (label, label_resolved) = match inputs.localisation.get(&loc_key) {
            Some(name) => (name.clone(), true),
            None => (loc_key, false),
        };
        markers.push(FurnitureMarker {
            province_id: *province_id,
            state_id: *state_id,
            owner,
            anchor_x: (sum_x / count as f64) as f32,
            anchor_y: (sum_y / count as f64) as f32,
            vp_value: *vp_value,
            label,
            label_resolved,
            kind: if is_capital {
                FurnitureMarkerKind::Capital
            } else {
                FurnitureMarkerKind::VictoryPoint
            },
            dot_tier: inputs
                .vp_icon_defines
                .map(|defines| defines.tier_for_vp(*vp_value))
                .unwrap_or(0),
            pixel_count: count,
        });
    }
    markers.sort_by_key(|marker| marker.province_id);
    markers
}

pub fn capital_states_from_resolver(resolver: &GameFileResolver) -> Result<BTreeMap<String, u32>> {
    let files = resolver.resolve_dir_files_recursive("history/countries", Some("txt"))?;
    let mut out = BTreeMap::new();
    for (rel, path) in &files {
        let Some(tag) = country_history_tag_from_rel(rel) else {
            continue;
        };
        let text = strip_comments(&read_latin1(path)?);
        if let Some(capital) = parse_capital_state(&text) {
            out.entry(tag).or_insert(capital);
        }
    }
    Ok(out)
}

pub fn ruling_parties_from_resolver(
    resolver: &GameFileResolver,
) -> Result<BTreeMap<String, String>> {
    let files = resolver.resolve_dir_files_recursive("history/countries", Some("txt"))?;
    let mut out = BTreeMap::new();
    for (rel, path) in &files {
        let Some(tag) = country_history_tag_from_rel(rel) else {
            continue;
        };
        let text = strip_comments(&read_latin1(path)?);
        if let Some(party) = parse_word_value(&text, "ruling_party") {
            out.entry(tag).or_insert(party);
        }
    }
    Ok(out)
}

pub fn country_colors_from_game_resolver(
    resolver: &GameFileResolver,
) -> Result<BTreeMap<String, [u8; 3]>> {
    crate::load_country_colors_from_game_resolver(resolver)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveDlcPresentation {
    display_names: Vec<String>,
    membership: BTreeSet<String>,
}

impl ActiveDlcPresentation {
    pub fn from_session_metadata(session: &SessionMetadataView) -> Result<Self> {
        if session.status != ViewStatus::Present {
            return Err(format!(
                "session metadata is not present ({:?}); active DLC presentation is unavailable",
                session.status
            )
            .into());
        }
        let display_names =
            match &session.active_dlcs {
                Leaf::Measured(names) => names.clone(),
                Leaf::Absent => {
                    return Err(
                        "session.active_dlcs is absent; refusing the historical all-DLC fallback"
                            .into(),
                    )
                }
                Leaf::Unobtained => return Err(
                    "session.active_dlcs is unobtained; refusing the historical all-DLC fallback"
                        .into(),
                ),
            };

        let mut membership = BTreeSet::new();
        for (index, name) in display_names.iter().enumerate() {
            if name.trim().is_empty() {
                return Err(format!("session.active_dlcs[{index}] is blank").into());
            }
            membership.insert(name.clone());
        }
        Ok(Self {
            display_names,
            membership,
        })
    }

    pub fn display_names(&self) -> &[String] {
        &self.display_names
    }

    pub fn has_dlc(&self, display_name: &str) -> bool {
        self.membership.contains(display_name)
    }
}

pub fn cosmetic_tags_from_resolver(
    resolver: &GameFileResolver,
    session: &SessionMetadataView,
) -> Result<BTreeMap<String, String>> {
    let presentation = ActiveDlcPresentation::from_session_metadata(session)?;
    cosmetic_tags_from_resolver_with_dlc(resolver, |dlc| presentation.has_dlc(dlc))
}

pub fn country_colors_from_game_resolver_with_session_metadata(
    resolver: &GameFileResolver,
    session: &SessionMetadataView,
) -> Result<BTreeMap<String, [u8; 3]>> {
    let cosmetic_tags = cosmetic_tags_from_resolver(resolver, session)?;
    crate::country_colors_from_game_resolver_with_cosmetics(resolver, &cosmetic_tags)
}

pub fn cosmetic_tags_from_resolver_with_dlc(
    resolver: &GameFileResolver,
    has_dlc: impl Fn(&str) -> bool,
) -> Result<BTreeMap<String, String>> {
    let files = resolver.resolve_dir_files_recursive("history/countries", Some("txt"))?;
    let mut out = crate::cosmetic_tags_1936_from_history_files(&files)?;
    let conditional = crate::dlc_conditional_cosmetics_1936_from_history_files(&files)?;
    for (tag, entries) in conditional {
        for entry in entries {
            let holds = if entry.negated {
                !has_dlc(&entry.dlc)
            } else {
                has_dlc(&entry.dlc)
            };
            if holds {
                out.insert(tag.clone(), entry.cosmetic);
            }
        }
    }
    Ok(out)
}

pub(crate) fn country_history_tag_from_rel(rel: &str) -> Option<String> {
    let file = rel.rsplit('/').next().unwrap_or(rel);
    let stem = file
        .strip_suffix(".txt")
        .or_else(|| file.strip_suffix(".TXT"))?;
    let token: String = stem
        .chars()
        .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        .collect();
    is_tag(&token).then_some(token)
}

pub(crate) fn parse_capital_state(text: &str) -> Option<u32> {
    parse_u32_value(text, "capital")
}

pub fn furniture_layer_with_keying(
    resolver: &GameFileResolver,
    language: &str,
    keying: &KeyingTables,
) -> Result<FurnitureLayer> {
    furniture_layer_from_resolver_keyed(resolver, language, Some(&keying.state_owners))
}

pub fn furniture_layer_from_resolver(
    resolver: &GameFileResolver,
    language: &str,
) -> Result<FurnitureLayer> {
    furniture_layer_from_resolver_keyed(resolver, language, None)
}

fn furniture_layer_from_resolver_keyed(
    resolver: &GameFileResolver,
    language: &str,
    owner_override: Option<&BTreeMap<u32, String>>,
) -> Result<FurnitureLayer> {
    let definition_csv = resolver.resolve_required("map/definition.csv", "definition.csv")?;
    let provinces_bmp = resolver.resolve_required("map/provinces.bmp", "provinces.bmp")?;
    let (_definitions, color_to_id) = load_definition_csv(&definition_csv)?;
    let bmp = load_provinces_bmp(&provinces_bmp, &color_to_id)?;

    let state_files = resolver.resolve_dir_files("history/states", Some("txt"))?;
    let (states, province_to_state, static_owners) = load_states(&state_files)?;
    let state_owners = owner_override.cloned().unwrap_or(static_owners);
    let mut state_victory_points = BTreeMap::new();
    for (state_id, meta) in &states {
        if !meta.victory_points.is_empty() {
            state_victory_points.insert(*state_id, meta.victory_points.clone());
        }
    }

    let capital_states = capital_states_from_resolver(resolver)?;
    let localisation = localisation_table_from_resolver(resolver, language)?;
    let vp_icon_defines = vp_icon_defines_from_resolver(resolver)?;

    let markers = compute_furniture_markers(&FurnitureLawInputs {
        province_ids: &bmp.ids,
        width: bmp.width,
        height: bmp.height,
        province_to_state: &province_to_state,
        state_owners: &state_owners,
        state_victory_points: &state_victory_points,
        capital_states: &capital_states,
        localisation: &localisation,
        vp_icon_defines: Some(&vp_icon_defines),
    });

    Ok(FurnitureLayer {
        width: bmp.width,
        height: bmp.height,
        province_ids: bmp.ids,
        province_to_state,
        markers,
        localisation_key_count: localisation.len(),
        capital_count: capital_states.len(),
        vp_icon_defines,
    })
}

pub struct FurnitureLayer {
    pub width: usize,
    pub height: usize,
    pub province_ids: Vec<u32>,
    pub province_to_state: BTreeMap<u32, u32>,
    pub markers: Vec<FurnitureMarker>,
    pub localisation_key_count: usize,
    pub capital_count: usize,
    pub vp_icon_defines: VpIconDefines,
}

impl FurnitureLayer {
    pub fn land_mask(&self) -> Vec<bool> {
        let mut mask = vec![false; self.width * self.height];
        for (i, &pid) in self.province_ids.iter().enumerate() {
            if pid != 0 && self.province_to_state.contains_key(&pid) {
                mask[i] = true;
            }
        }
        mask
    }
}


#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BmFontGlyph {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub xoffset: i32,
    pub yoffset: i32,
    pub xadvance: i32,
    pub page: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BmFont {
    pub line_height: u32,
    pub base: u32,
    pub scale_w: u32,
    pub scale_h: u32,
    pub page_count: u32,
    pub glyphs: HashMap<u32, BmFontGlyph>,
    pub kernings: HashMap<(u32, u32), i32>,
}

impl BmFont {
    pub fn glyph(&self, code: u32) -> Option<&BmFontGlyph> {
        self.glyphs.get(&code)
    }


    pub fn measure(&self, text: &str) -> i32 {
        let mut width = 0;
        let mut previous = None;
        for ch in text.chars() {
            let id = ch as u32;
            let Some(glyph) = self.glyphs.get(&id) else {
                previous = None;
                continue;
            };
            if let Some(previous) = previous {
                width += self.kernings.get(&(previous, id)).copied().unwrap_or(0);
            }
            width += glyph.xadvance;
            previous = Some(id);
        }
        width
    }

    pub fn label_text_metrics(&self, text: &str) -> Result<LabelTextMetrics> {
        let mut advances = Vec::with_capacity(text.chars().count());
        let mut stations = Vec::with_capacity(text.chars().count());
        let mut pen = 0.0f32;
        let mut previous = None;
        for (slot_index, ch) in text.chars().enumerate() {
            let id = ch as u32;
            let glyph = self
                .glyphs
                .get(&id)
                .ok_or_else(|| format!("map font is missing glyph {ch:?} in {text:?}"))?;
            let kerning = previous
                .and_then(|previous| self.kernings.get(&(previous, id)))
                .copied()
                .unwrap_or(0);
            pen += kerning as f32;
            if glyph.width > 0 && glyph.height > 0 {
                stations.push(MapLabelStraightLayoutStation {
                    slot_index,
                    x: pen + glyph.xoffset as f32 + glyph.width as f32 * 0.5,
                    y: glyph.yoffset as f32,
                });
            }
            advances.push(((kerning + glyph.xadvance) as f32, ch == ' '));
            pen += glyph.xadvance as f32;
            previous = Some(id);
        }
        let space_advance_px = self
            .glyphs
            .get(&u32::from(b' '))
            .map(|glyph| glyph.xadvance as f32)
            .ok_or("map font has no space glyph for engine tracking")?;
        Ok(LabelTextMetrics {
            advances,
            line_height_px: self.line_height as f32,
            space_advance_px,
            straight_layout_stations: stations,
        })
    }

    pub fn layout_mapname_tracked(
        &self,
        text: &str,
        tracking: f32,
    ) -> Result<Vec<MapLabelGlyphLayoutQuad>> {
        let mut quads = Vec::new();
        let mut pen = 0.0f32;
        let mut previous = None;
        for ch in text.chars() {
            let id = ch as u32;
            let glyph = self
                .glyphs
                .get(&id)
                .ok_or_else(|| format!("map font is missing glyph {ch:?} in {text:?}"))?;
            if let Some(previous) = previous {
                pen += self.kernings.get(&(previous, id)).copied().unwrap_or(0) as f32;
            }
            if glyph.width > 0 && glyph.height > 0 {
                let x0 = pen + glyph.xoffset as f32;
                let y0 = glyph.yoffset as f32;
                quads.push(MapLabelGlyphLayoutQuad {
                    x0,
                    y0,
                    x1: x0 + glyph.width as f32,
                    y1: y0 + glyph.height as f32,
                    u0: glyph.x as f32 / self.scale_w as f32,
                    v0: glyph.y as f32 / self.scale_h as f32,
                    u1: (glyph.x + glyph.width) as f32 / self.scale_w as f32,
                    v1: (glyph.y + glyph.height) as f32 / self.scale_h as f32,
                    measured_t: 0.0,
                });
            }
            pen += glyph.xadvance as f32 + tracking;
            previous = Some(id);
        }
        let measured = mapname_measured_glyph_parameters(&quads)
            .ok_or("map font produced invalid straight-layout stations")?;
        for (quad, measured_t) in quads.iter_mut().zip(measured) {
            quad.measured_t = measured_t;
        }
        Ok(quads)
    }
}

pub fn parse_bmfont(text: &str) -> Result<BmFont> {
    let mut line_height = 0u32;
    let mut base = 0u32;
    let mut scale_w = 0u32;
    let mut scale_h = 0u32;
    let mut page_count = 0u32;
    let mut glyphs = HashMap::new();
    let mut kernings = HashMap::new();
    for line in text.lines() {
        let mut fields = bmfont_fields(line);
        let head = line.split_whitespace().next().unwrap_or("");
        match head {
            "common" => {
                line_height = fields
                    .remove("lineHeight")
                    .and_then(parse_u32_field)
                    .unwrap_or(0);
                base = fields.remove("base").and_then(parse_u32_field).unwrap_or(0);
                scale_w = fields
                    .remove("scaleW")
                    .and_then(parse_u32_field)
                    .unwrap_or(0);
                scale_h = fields
                    .remove("scaleH")
                    .and_then(parse_u32_field)
                    .unwrap_or(0);
                page_count = fields
                    .remove("pages")
                    .and_then(parse_u32_field)
                    .unwrap_or(0);
            }
            "char" => {
                let Some(id) = fields.get("id").and_then(|v| parse_u32_field(v.clone())) else {
                    continue;
                };
                let glyph = BmFontGlyph {
                    x: fields
                        .get("x")
                        .and_then(|v| parse_u32_field(v.clone()))
                        .unwrap_or(0),
                    y: fields
                        .get("y")
                        .and_then(|v| parse_u32_field(v.clone()))
                        .unwrap_or(0),
                    width: fields
                        .get("width")
                        .and_then(|v| parse_u32_field(v.clone()))
                        .unwrap_or(0),
                    height: fields
                        .get("height")
                        .and_then(|v| parse_u32_field(v.clone()))
                        .unwrap_or(0),
                    xoffset: fields
                        .get("xoffset")
                        .and_then(|v| v.parse::<i32>().ok())
                        .unwrap_or(0),
                    yoffset: fields
                        .get("yoffset")
                        .and_then(|v| v.parse::<i32>().ok())
                        .unwrap_or(0),
                    xadvance: fields
                        .get("xadvance")
                        .and_then(|v| v.parse::<i32>().ok())
                        .unwrap_or(0),
                    page: fields
                        .get("page")
                        .and_then(|v| parse_u32_field(v.clone()))
                        .unwrap_or(0),
                };
                glyphs.insert(id, glyph);
            }
            "kerning" => {
                let Some(first) = fields
                    .get("first")
                    .and_then(|value| parse_u32_field(value.clone()))
                else {
                    continue;
                };
                let Some(second) = fields
                    .get("second")
                    .and_then(|value| parse_u32_field(value.clone()))
                else {
                    continue;
                };
                let amount = fields
                    .get("amount")
                    .and_then(|value| value.parse::<i32>().ok())
                    .unwrap_or(0);
                kernings.insert((first, second), amount);
            }
            _ => {}
        }
    }
    if glyphs.is_empty()
        || line_height == 0
        || scale_w == 0
        || scale_h == 0
        || page_count == 0
        || glyphs.values().any(|glyph| glyph.page >= page_count)
    {
        return Err("bmfont has no glyphs or valid common metrics".into());
    }
    Ok(BmFont {
        line_height,
        base,
        scale_w,
        scale_h,
        page_count,
        glyphs,
        kernings,
    })
}

pub fn load_vp_label_font(resolver: &GameFileResolver) -> Result<(BmFont, RgbaFontAtlas)> {
    let fnt_path = resolver.resolve_required(HOI_20BS_FNT_SOURCE, "hoi_20bs.fnt")?;
    let font = parse_bmfont(&read_latin1(&fnt_path)?)?;
    let atlas_path = resolver.resolve_required(HOI_20BS_ATLAS_SOURCE, "hoi_20bs.dds")?;
    let image = load_dds_rgba(&atlas_path)?;
    Ok((
        font,
        RgbaFontAtlas {
            width: image.width,
            height: image.height,
            pixels: image.pixels,
        },
    ))
}

#[derive(Clone, Debug)]
pub struct RgbaFontAtlas {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

impl RgbaFontAtlas {
    pub fn coverage(&self, x: usize, y: usize) -> u8 {
        if x >= self.width || y >= self.height {
            return 0;
        }
        let i = (y * self.width + x) * 4;
        let r = self.pixels[i];
        let g = self.pixels[i + 1];
        let b = self.pixels[i + 2];
        let a = self.pixels[i + 3];
        r.max(g).max(b).max(a)
    }
}

fn bmfont_fields(line: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for token in split_bmfont_tokens(line) {
        if let Some((key, value)) = token.split_once('=') {
            out.insert(key.to_string(), value.trim_matches('"').to_string());
        }
    }
    out
}

/// Tokenize a BMFont line on whitespace but keep quoted spans together (so
/// `face="Ubuntu Light"` is one token). No allocations beyond the token
/// strings.
fn split_bmfont_tokens(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for ch in line.chars() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                current.push(ch);
            }
            c if c.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn parse_u32_field(value: String) -> Option<u32> {
    value.trim().parse::<u32>().ok()
}

pub fn vp_strip_texture_path(resolver: &GameFileResolver) -> Result<Option<PathBuf>> {
    let asset = crate::map_icon_runtime::load_vp_strip_asset(resolver)?;
    Ok(Some(resolver.resolve_required(
        &asset.texture_file,
        VP_STRIP_SPRITE_NAME,
    )?))
}


pub const AIR_BASE_BUILDING: &str = "air_base";
pub const NAVAL_BASE_BUILDING: &str = "naval_base";
pub const NAVAL_BASE_SPAWN_LOCATOR: &str = "naval_base_spawn";

pub const AIR_BASE_SPRITE_NAME: &str = "GFX_mapicon_air_base";
pub const AIR_BASE_SPRITE_TEXTURE: &str = "gfx/interface/mapicons/mapicon_building_airbase.dds";
pub const AIR_BASE_SPRITE_FRAMES: u32 = 7;
pub const NAVAL_BASE_SPRITE_NAME: &str = "GFX_mapicon_naval_base";
pub const NAVAL_BASE_SPRITE_TEXTURE: &str = "gfx/interface/mapicons/mapicon_building_navalbase.dds";
pub const NAVAL_BASE_SPRITE_FRAMES: u32 = 11;

pub const AIRBASE_ICON_DISTANCE_CUTOFF_KEY: &str = "AIRBASE_ICON_DISTANCE_CUTOFF";
pub const NAVALBASE_ICON_DISTANCE_CUTOFF_KEY: &str = "NAVALBASE_ICON_DISTANCE_CUTOFF";

pub const AIR_BASE_STATE_MAX_LEVEL: u32 = 10;
pub const NAVAL_BASE_PROVINCE_MAX_LEVEL: u32 = 10;

pub const NAVAL_SPAWN_LAND_SNAP_RADIUS: i32 = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaseIconKind {
    AirBase,
    NavalBase,
}

impl BaseIconKind {
    pub fn as_str(self) -> &'static str {
        match self {
            BaseIconKind::AirBase => "air_base",
            BaseIconKind::NavalBase => "naval_base",
        }
    }
    pub fn sprite_frames(self) -> u32 {
        match self {
            BaseIconKind::AirBase => AIR_BASE_SPRITE_FRAMES,
            BaseIconKind::NavalBase => NAVAL_BASE_SPRITE_FRAMES,
        }
    }
    pub fn sprite_name(self) -> &'static str {
        match self {
            BaseIconKind::AirBase => AIR_BASE_SPRITE_NAME,
            BaseIconKind::NavalBase => NAVAL_BASE_SPRITE_NAME,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BaseIconDefines {
    pub airbase_cutoff: f32,
    pub navalbase_cutoff: f32,
}

impl BaseIconDefines {
    pub fn cutoff(&self, kind: BaseIconKind) -> f32 {
        match kind {
            BaseIconKind::AirBase => self.airbase_cutoff,
            BaseIconKind::NavalBase => self.navalbase_cutoff,
        }
    }
    pub fn visible(&self, kind: BaseIconKind, cam_dist: f32) -> bool {
        cam_dist <= self.cutoff(kind)
    }
}

pub fn parse_base_icon_defines(source: &str) -> Result<BaseIconDefines> {
    Ok(BaseIconDefines {
        airbase_cutoff: parse_lua_number_define(source, AIRBASE_ICON_DISTANCE_CUTOFF_KEY)?,
        navalbase_cutoff: parse_lua_number_define(source, NAVALBASE_ICON_DISTANCE_CUTOFF_KEY)?,
    })
}

pub fn base_icon_defines_from_resolver(resolver: &GameFileResolver) -> Result<BaseIconDefines> {
    let path = resolver.resolve_required(BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE, "graphics defines lua")?;
    let source = latin1_bytes_to_string(&std::fs::read(&path)?);
    parse_base_icon_defines(&source)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StateBuildingLevels {
    pub air_base: u32,
    pub naval_base: BTreeMap<u32, u32>,
}

pub fn parse_state_building_levels(text: &str) -> Option<StateBuildingLevels> {
    let (buildings, _) = extract_block_after(text, "buildings", 0)?;
    let air_base = parse_u32_value(&buildings, AIR_BASE_BUILDING).unwrap_or(0);
    let mut naval_base = BTreeMap::new();
    for (name, block) in top_level_assignment_blocks(&buildings) {
        let Ok(province_id) = name.trim().parse::<u32>() else {
            continue;
        };
        if let Some(level) = parse_u32_value(&block, NAVAL_BASE_BUILDING) {
            if level >= 1 {
                naval_base.insert(province_id, level);
            }
        }
    }
    Some(StateBuildingLevels {
        air_base,
        naval_base,
    })
}

pub fn state_building_levels_from_resolver(
    resolver: &GameFileResolver,
) -> Result<(BTreeMap<u32, u32>, BTreeMap<u32, u32>)> {
    let files = resolver.resolve_dir_files("history/states", Some("txt"))?;
    let mut air_by_state = BTreeMap::new();
    let mut naval_by_province = BTreeMap::new();
    for (_, path) in &files {
        let text = strip_comments(&read_latin1(path)?);
        let Some(state_id) = parse_u32_value(&text, "id") else {
            continue;
        };
        let Some(levels) = parse_state_building_levels(&text) else {
            continue;
        };
        if levels.air_base >= 1 {
            air_by_state.insert(state_id, levels.air_base);
        }
        for (province_id, level) in levels.naval_base {
            naval_by_province.insert(province_id, level);
        }
    }
    Ok((air_by_state, naval_by_province))
}

pub fn building_locator_pixel(pos: [f32; 3], map_height: usize) -> (f32, f32) {
    let x = pos[0];
    let y = (map_height.saturating_sub(1) as f32) - pos[2];
    (x, y)
}

pub fn associate_naval_spawns_to_provinces(
    placements: &[MapBuildingPlacement],
    province_ids: &[u32],
    width: usize,
    height: usize,
    province_to_state: &BTreeMap<u32, u32>,
) -> BTreeMap<u32, (f32, f32)> {
    let mut best: BTreeMap<u32, (i32, (f32, f32))> = BTreeMap::new();
    if province_ids.len() != width.saturating_mul(height) || width == 0 || height == 0 {
        return BTreeMap::new();
    }
    let land_at = |px: i32, py: i32| -> Option<u32> {
        if px < 0 || py < 0 || px >= width as i32 || py >= height as i32 {
            return None;
        }
        let pid = province_ids[py as usize * width + px as usize];
        if pid != 0 && province_to_state.contains_key(&pid) {
            Some(pid)
        } else {
            None
        }
    };
    for placement in placements {
        if placement.building != NAVAL_BASE_SPAWN_LOCATOR {
            continue;
        }
        let (fx, fy) = building_locator_pixel(placement.pos, height);
        let cx = fx.round() as i32;
        let cy = fy.round() as i32;
        let mut found: Option<(u32, i32)> = None;
        'search: for r in 0..=NAVAL_SPAWN_LAND_SNAP_RADIUS {
            for dy in -r..=r {
                for dx in -r..=r {
                    if r > 0 && dx.abs() != r && dy.abs() != r {
                        continue;
                    }
                    if let Some(pid) = land_at(cx + dx, cy + dy) {
                        found = Some((pid, r));
                        break 'search;
                    }
                }
            }
        }
        let Some((pid, dist)) = found else {
            continue;
        };
        match best.get(&pid) {
            Some((best_dist, _)) if *best_dist <= dist => {}
            _ => {
                best.insert(pid, (dist, (fx, fy)));
            }
        }
    }
    best.into_iter()
        .map(|(pid, (_, pixel))| (pid, pixel))
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct BaseIconMarker {
    pub kind: BaseIconKind,
    pub state_id: u32,
    pub province_id: Option<u32>,
    pub owner: String,
    pub anchor_x: f32,
    pub anchor_y: f32,
    pub level: u32,
    pub frame: u32,
}

pub struct BaseIconInputs<'a> {
    pub placements: &'a [MapBuildingPlacement],
    pub province_ids: &'a [u32],
    pub width: usize,
    pub height: usize,
    pub province_to_state: &'a BTreeMap<u32, u32>,
    pub state_owners: &'a BTreeMap<u32, String>,
    pub air_base_levels: &'a BTreeMap<u32, u32>,
    pub naval_base_levels: &'a BTreeMap<u32, u32>,
    pub show_on_map: &'a std::collections::BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BaseIconLayer {
    pub markers: Vec<BaseIconMarker>,
    pub air_base_count: usize,
    pub naval_base_count: usize,
    pub naval_unattributed: Vec<u32>,
}

pub fn base_icon_frame(kind: BaseIconKind, level: u32) -> u32 {
    level.min(kind.sprite_frames().saturating_sub(1))
}

pub fn compute_base_icon_markers(inputs: &BaseIconInputs) -> BaseIconLayer {
    let mut markers = Vec::new();
    let air_on_map = inputs.show_on_map.contains(AIR_BASE_BUILDING);
    let naval_on_map = inputs.show_on_map.contains(NAVAL_BASE_BUILDING);

    if air_on_map {
        let mut air_locator: BTreeMap<u32, (f32, f32)> = BTreeMap::new();
        for placement in inputs.placements {
            if placement.building == AIR_BASE_BUILDING {
                air_locator
                    .entry(placement.state_id)
                    .or_insert_with(|| building_locator_pixel(placement.pos, inputs.height));
            }
        }
        for (state_id, level) in inputs.air_base_levels {
            if *level < 1 {
                continue;
            }
            let Some((ax, ay)) = air_locator.get(state_id).copied() else {
                continue;
            };
            markers.push(BaseIconMarker {
                kind: BaseIconKind::AirBase,
                state_id: *state_id,
                province_id: None,
                owner: inputs
                    .state_owners
                    .get(state_id)
                    .cloned()
                    .unwrap_or_default(),
                anchor_x: ax,
                anchor_y: ay,
                level: *level,
                frame: base_icon_frame(BaseIconKind::AirBase, *level),
            });
        }
    }
    let air_base_count = markers.len();

    let mut naval_unattributed = Vec::new();
    if naval_on_map {
        let spawn_pixels = associate_naval_spawns_to_provinces(
            inputs.placements,
            inputs.province_ids,
            inputs.width,
            inputs.height,
            inputs.province_to_state,
        );
        for (province_id, level) in inputs.naval_base_levels {
            if *level < 1 {
                continue;
            }
            let Some((nx, ny)) = spawn_pixels.get(province_id).copied() else {
                naval_unattributed.push(*province_id);
                continue;
            };
            let state_id = inputs
                .province_to_state
                .get(province_id)
                .copied()
                .unwrap_or(0);
            markers.push(BaseIconMarker {
                kind: BaseIconKind::NavalBase,
                state_id,
                province_id: Some(*province_id),
                owner: inputs
                    .state_owners
                    .get(&state_id)
                    .cloned()
                    .unwrap_or_default(),
                anchor_x: nx,
                anchor_y: ny,
                level: *level,
                frame: base_icon_frame(BaseIconKind::NavalBase, *level),
            });
        }
    }
    let naval_base_count = markers.len() - air_base_count;
    naval_unattributed.sort_unstable();

    BaseIconLayer {
        markers,
        air_base_count,
        naval_base_count,
        naval_unattributed,
    }
}

pub fn base_icon_layer_from_resolver(resolver: &GameFileResolver) -> Result<BaseIconLayer> {
    let definition_csv = resolver.resolve_required("map/definition.csv", "definition.csv")?;
    let provinces_bmp = resolver.resolve_required("map/provinces.bmp", "provinces.bmp")?;
    let (_definitions, color_to_id) = load_definition_csv(&definition_csv)?;
    let bmp = load_provinces_bmp(&provinces_bmp, &color_to_id)?;

    let state_files = resolver.resolve_dir_files("history/states", Some("txt"))?;
    let (_states, province_to_state, state_owners) = load_states(&state_files)?;
    let (air_base_levels, naval_base_levels) = state_building_levels_from_resolver(resolver)?;

    let placements = map_building_placements_from_resolver(resolver)?;
    let show_on_map = show_on_map_buildings_from_resolver(resolver)?
        .into_iter()
        .filter(|building| building.show_on_map >= 1)
        .map(|building| building.name)
        .collect::<std::collections::BTreeSet<_>>();

    Ok(compute_base_icon_markers(&BaseIconInputs {
        placements: &placements,
        province_ids: &bmp.ids,
        width: bmp.width,
        height: bmp.height,
        province_to_state: &province_to_state,
        state_owners: &state_owners,
        air_base_levels: &air_base_levels,
        naval_base_levels: &naval_base_levels,
        show_on_map: &show_on_map,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_test_dir;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::Path;

    fn write_text(root: &Path, rel: &str, text: &str) {
        let path = rel
            .split('/')
            .fold(root.to_path_buf(), |path, part| path.join(part));
        fs::create_dir_all(path.parent().expect("fixture parent")).expect("create fixture dir");
        fs::write(path, text).expect("write fixture file");
    }

    fn session_metadata(active_dlcs: Leaf<Vec<String>>) -> SessionMetadataView {
        SessionMetadataView {
            status: ViewStatus::Present,
            player_countries: Leaf::Absent,
            speed_events: Leaf::Absent,
            active_dlcs,
            localisation_language: Leaf::Absent,
            display_variant: Leaf::Absent,
        }
    }

    fn raster_rect(ids: &mut [u32], width: usize, id: u32, rect: (usize, usize, usize, usize)) {
        let (x0, y0, x1, y1) = rect;
        for y in y0..y1 {
            for x in x0..x1 {
                ids[y * width + x] = id;
            }
        }
    }

    fn base_inputs<'a>(
        province_ids: &'a [u32],
        width: usize,
        height: usize,
        province_to_state: &'a BTreeMap<u32, u32>,
        state_owners: &'a BTreeMap<u32, String>,
        state_victory_points: &'a BTreeMap<u32, BTreeMap<u32, i32>>,
        capital_states: &'a BTreeMap<String, u32>,
        localisation: &'a BTreeMap<String, String>,
    ) -> FurnitureLawInputs<'a> {
        FurnitureLawInputs {
            province_ids,
            width,
            height,
            province_to_state,
            state_owners,
            state_victory_points,
            capital_states,
            localisation,
            vp_icon_defines: None,
        }
    }

    fn sample_defines() -> VpIconDefines {
        VpIconDefines {
            levels: 3,
            map_icon_after: vec![0, 9, 20],
            text_cutoff: vec![150.0, 250.0, 500.0],
            text_cutoff_min: 100.0,
            text_cutoff_max: 800.0,
            dot_cutoff_min: 100.0,
            dot_cutoff_max: 1000.0,
            max_victory_points_for_percent: 22.0,
            capital_cutoff_max: 1000.0,
            capital_icon_cutoff: 1100.0,
        }
    }

    #[test]
    fn victory_point_marker_lands_on_province_centroid() {
        let width = 100;
        let height = 60;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 3838, (20, 10, 40, 30));
        let province_to_state = BTreeMap::from([(3838u32, 1u32)]);
        let state_owners = BTreeMap::from([(1u32, "FRA".to_string())]);
        let state_victory_points = BTreeMap::from([(1u32, BTreeMap::from([(3838u32, 1i32)]))]);
        let capital_states = BTreeMap::new();
        let localisation =
            BTreeMap::from([("VICTORY_POINTS_3838".to_string(), "Ajáccio".to_string())]);
        let inputs = base_inputs(
            &ids,
            width,
            height,
            &province_to_state,
            &state_owners,
            &state_victory_points,
            &capital_states,
            &localisation,
        );
        let markers = compute_furniture_markers(&inputs);
        assert_eq!(markers.len(), 1);
        let m = &markers[0];
        assert_eq!(m.province_id, 3838);
        assert_eq!(m.owner, "FRA");
        assert_eq!(m.vp_value, 1);
        assert_eq!(m.label, "Ajáccio");
        assert!(m.label_resolved);
        assert_eq!(m.kind, FurnitureMarkerKind::VictoryPoint);
        assert!((m.anchor_x - 29.5).abs() < 0.01, "anchor_x {}", m.anchor_x);
        assert!((m.anchor_y - 19.5).abs() < 0.01, "anchor_y {}", m.anchor_y);
    }

    #[test]
    fn capital_state_vp_becomes_capital_marker() {
        let width = 80;
        let height = 40;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 64, (10, 10, 30, 30));
        let province_to_state = BTreeMap::from([(64u32, 64u32)]);
        let state_owners = BTreeMap::from([(64u32, "GER".to_string())]);
        let state_victory_points = BTreeMap::from([(64u32, BTreeMap::from([(64u32, 40i32)]))]);
        let capital_states = BTreeMap::from([("GER".to_string(), 64u32)]);
        let localisation = BTreeMap::new();
        let inputs = base_inputs(
            &ids,
            width,
            height,
            &province_to_state,
            &state_owners,
            &state_victory_points,
            &capital_states,
            &localisation,
        );
        let markers = compute_furniture_markers(&inputs);
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].kind, FurnitureMarkerKind::Capital);
        assert_eq!(markers[0].label, "VICTORY_POINTS_64");
        assert!(!markers[0].label_resolved);
    }

    #[test]
    fn only_highest_vp_in_capital_state_is_starred() {
        let width = 120;
        let height = 40;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 7859, (10, 10, 30, 30));
        raster_rect(&mut ids, width, 12406, (60, 10, 90, 30));
        let province_to_state = BTreeMap::from([(7859u32, 285u32), (12406u32, 285u32)]);
        let state_owners = BTreeMap::from([(285u32, "AST".to_string())]);
        let state_victory_points = BTreeMap::from([(
            285u32,
            BTreeMap::from([(7859u32, 30i32), (12406u32, 20i32)]),
        )]);
        let capital_states = BTreeMap::from([("AST".to_string(), 285u32)]);
        let localisation = BTreeMap::new();
        let inputs = base_inputs(
            &ids,
            width,
            height,
            &province_to_state,
            &state_owners,
            &state_victory_points,
            &capital_states,
            &localisation,
        );
        let markers = compute_furniture_markers(&inputs);
        assert_eq!(markers.len(), 2);
        let canberra = markers.iter().find(|m| m.province_id == 7859).unwrap();
        let sydney = markers.iter().find(|m| m.province_id == 12406).unwrap();
        assert_eq!(canberra.kind, FurnitureMarkerKind::Capital);
        assert_eq!(
            sydney.kind,
            FurnitureMarkerKind::VictoryPoint,
            "only the highest-VP province in the capital state stars"
        );
        let tied = BTreeMap::from([(
            285u32,
            BTreeMap::from([(7859u32, 20i32), (12406u32, 20i32)]),
        )]);
        let mut tied_inputs = base_inputs(
            &ids,
            width,
            height,
            &province_to_state,
            &state_owners,
            &tied,
            &capital_states,
            &localisation,
        );
        tied_inputs.state_victory_points = &tied;
        let tied_markers = compute_furniture_markers(&tied_inputs);
        let starred: Vec<u32> = tied_markers
            .iter()
            .filter(|m| m.kind == FurnitureMarkerKind::Capital)
            .map(|m| m.province_id)
            .collect();
        assert_eq!(starred, vec![7859], "tie breaks to the lowest province id");
    }

    #[test]
    fn capital_state_with_no_vp_yields_no_capital_marker() {
        let width = 40;
        let height = 40;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 13272, (5, 5, 20, 20));
        let province_to_state = BTreeMap::from([(13272u32, 914u32)]);
        let state_owners = BTreeMap::from([(914u32, "JAN".to_string())]);
        let state_victory_points: BTreeMap<u32, BTreeMap<u32, i32>> = BTreeMap::new();
        let capital_states = BTreeMap::from([("JAN".to_string(), 914u32)]);
        let localisation = BTreeMap::new();
        let inputs = base_inputs(
            &ids,
            width,
            height,
            &province_to_state,
            &state_owners,
            &state_victory_points,
            &capital_states,
            &localisation,
        );
        assert!(
            compute_furniture_markers(&inputs).is_empty(),
            "a capital state with no VP must not fabricate a capital marker"
        );
    }

    #[test]
    fn vp_province_absent_from_raster_fails_closed() {
        let width = 40;
        let height = 40;
        let ids = vec![0u32; width * height];
        let province_to_state = BTreeMap::from([(999u32, 1u32)]);
        let state_owners = BTreeMap::from([(1u32, "FRA".to_string())]);
        let state_victory_points = BTreeMap::from([(1u32, BTreeMap::from([(999u32, 5i32)]))]);
        let capital_states = BTreeMap::new();
        let localisation = BTreeMap::new();
        let inputs = base_inputs(
            &ids,
            width,
            height,
            &province_to_state,
            &state_owners,
            &state_victory_points,
            &capital_states,
            &localisation,
        );
        assert!(
            compute_furniture_markers(&inputs).is_empty(),
            "a VP province with no raster pixels must not place a marker"
        );
    }

    #[test]
    fn markers_are_deterministic_and_sorted_by_province() {
        let width = 120;
        let height = 40;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 50, (10, 10, 30, 30));
        raster_rect(&mut ids, width, 20, (60, 10, 90, 30));
        let province_to_state = BTreeMap::from([(50u32, 5u32), (20u32, 2u32)]);
        let state_owners = BTreeMap::from([(5u32, "SOV".to_string()), (2u32, "GER".to_string())]);
        let state_victory_points = BTreeMap::from([
            (5u32, BTreeMap::from([(50u32, 3i32)])),
            (2u32, BTreeMap::from([(20u32, 7i32)])),
        ]);
        let capital_states = BTreeMap::new();
        let localisation = BTreeMap::new();
        let inputs = base_inputs(
            &ids,
            width,
            height,
            &province_to_state,
            &state_owners,
            &state_victory_points,
            &capital_states,
            &localisation,
        );
        let a = compute_furniture_markers(&inputs);
        let b = compute_furniture_markers(&inputs);
        assert_eq!(a, b, "law must be deterministic");
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].province_id, 20, "sorted by province id");
        assert_eq!(a[1].province_id, 50);
    }

    #[test]
    fn victory_points_parse_on_real_history_states_row_formats() {
        let text = strip_comments(
            "state={ id=1 name=\"STATE_1\"\n history={ owner = FRA\n victory_points = { 3838 1 }\n } provinces={ 3838 9851 } }",
        );
        assert!(text.contains("victory_points"));
        let multi = strip_comments(
            "state={ id=2 history={ victory_points = { 100 5 } victory_points = { 200 12 } } }",
        );
        assert!(multi.matches("victory_points").count() == 2);
    }

    #[test]
    fn capital_tag_extraction_handles_shipped_filename_shapes() {
        assert_eq!(
            country_history_tag_from_rel("history/countries/GER - Germany.txt").as_deref(),
            Some("GER")
        );
        assert_eq!(
            country_history_tag_from_rel("history/countries/USA - USA.txt").as_deref(),
            Some("USA")
        );
        assert_eq!(
            country_history_tag_from_rel("history/countries/HBC  - Hebei-Chahar.txt").as_deref(),
            Some("HBC")
        );
        assert_eq!(
            country_history_tag_from_rel("history/countries/KLT -Kalat.txt").as_deref(),
            Some("KLT")
        );
        assert_eq!(
            country_history_tag_from_rel("history/countries/D01 - Dynamic.txt").as_deref(),
            Some("D01")
        );
        assert_eq!(
            country_history_tag_from_rel("history/countries/readme.txt"),
            None
        );
    }

    #[test]
    fn capital_parse_is_word_boundary_aware() {
        let text = strip_comments(
            "set_variable = { SOV_propaganda_anti_capitalism_cost = 1 }\ncapital = 219\n",
        );
        assert_eq!(parse_capital_state(&text), Some(219));
        let bom = strip_comments("\u{feff}capital = 361\n");
        assert_eq!(parse_capital_state(&bom), Some(361));
        assert_eq!(parse_capital_state("owner = FRA\n"), None);
    }

    #[test]
    fn bmfont_parses_metrics_page_and_kerning_shape() {
        let fnt = "info face=\"Ubuntu Light\" size=20 bold=1\n\
                   common lineHeight=20 base=16 scaleW=512 scaleH=256 pages=1\n\
                   page id=0 file=\"hoi_20b_0.dds\"\n\
                   chars count=2\n\
                   char id=76   x=189   y=106   width=13    height=14    xoffset=-2    yoffset=4     xadvance=8     page=0  chnl=15\n\
                   char id=111  x=235   y=119   width=13    height=12    xoffset=-2    yoffset=6     xadvance=9     page=0  chnl=15\n\
                   kernings count=1\n\
                   kerning first=76 second=111 amount=-1\n";
        let font = parse_bmfont(fnt).expect("parse bmfont");
        assert_eq!(font.line_height, 20);
        assert_eq!(font.base, 16);
        assert_eq!(font.scale_w, 512);
        assert_eq!(font.scale_h, 256);
        let l = font.glyph(76).expect("L glyph");
        assert_eq!((l.x, l.y, l.width, l.height), (189, 106, 13, 14));
        assert_eq!((l.xoffset, l.yoffset, l.xadvance), (-2, 4, 8));
        assert_eq!(l.page, 0);
        assert_eq!(font.page_count, 1);
        assert_eq!(font.measure("Lo"), 16);
    }

    #[test]
    fn bmfont_empty_fails_closed() {
        assert!(parse_bmfont("info face=\"x\"\ncommon lineHeight=20\n").is_err());
    }

    #[test]
    fn vp_icon_defines_parse_from_lua_block() {
        let lua = "\
NDefines = {\n\
\tNGraphics = {\n\
\t\tVICTORY_POINT_LEVELS = 3,\n\
\t\tVICTORY_POINT_MAP_ICON_AFTER = {0, 9, 20}, -- becomes bigger dot\n\
\t\tVICTORY_POINT_MAP_ICON_CAPITAL_CUTOFF_MAX = 1000.0,\n\
\t\tVICTORY_POINT_MAP_ICON_TEXT_CUTOFF = {150, 250, 500},\n\
\t\tVICTORY_POINT_MAP_ICON_TEXT_CUTOFF_MIN = 100.0,\n\
\t\tVICTORY_POINT_MAP_ICON_TEXT_CUTOFF_MAX = 800.0,\n\
\t\tVICTORY_POINT_MAP_ICON_DOT_CUTOFF_MIN = 100.0,\n\
\t\tVICTORY_POINT_MAP_ICON_DOT_CUTOFF_MAX = 1000.0,\n\
\t\tVICTORY_POINT_MAP_ICON_MAX_VICTORY_POINTS_FOR_PERCENT = 22,\n\
\t\tCAPITAL_ICON_CUTOFF = 1100,\n\
\t},\n\
}\n";
        let defines = parse_vp_icon_defines(lua).expect("parse vp icon defines");
        assert_eq!(defines, sample_defines());
    }

    #[test]
    fn vp_icon_defines_missing_key_fails_closed() {
        let lua = "\
VICTORY_POINT_LEVELS = 3,\n\
VICTORY_POINT_MAP_ICON_AFTER = {0, 9, 20},\n\
VICTORY_POINT_MAP_ICON_CAPITAL_CUTOFF_MAX = 1000.0,\n\
VICTORY_POINT_MAP_ICON_TEXT_CUTOFF = {150, 250, 500},\n\
VICTORY_POINT_MAP_ICON_TEXT_CUTOFF_MIN = 100.0,\n\
VICTORY_POINT_MAP_ICON_TEXT_CUTOFF_MAX = 800.0,\n\
VICTORY_POINT_MAP_ICON_DOT_CUTOFF_MIN = 100.0,\n\
VICTORY_POINT_MAP_ICON_DOT_CUTOFF_MAX = 1000.0,\n\
VICTORY_POINT_MAP_ICON_MAX_VICTORY_POINTS_FOR_PERCENT = 22,\n";
        assert!(parse_vp_icon_defines(lua).is_err());
    }

    #[test]
    fn dot_tier_follows_map_icon_after_thresholds() {
        let defines = sample_defines();
        assert_eq!(defines.tier_for_vp(0), 0);
        assert_eq!(defines.tier_for_vp(8), 0);
        assert_eq!(defines.tier_for_vp(9), 1);
        assert_eq!(defines.tier_for_vp(19), 1);
        assert_eq!(defines.tier_for_vp(20), 2);
        assert_eq!(defines.tier_for_vp(50), 2);
        assert!(defines.tier_for_vp(9999) < defines.levels);
    }

    #[test]
    fn marker_carries_file_derived_dot_tier() {
        let width = 60;
        let height = 40;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 64, (10, 10, 30, 30));
        let province_to_state = BTreeMap::from([(64u32, 64u32)]);
        let state_owners = BTreeMap::from([(64u32, "GER".to_string())]);
        let state_victory_points = BTreeMap::from([(64u32, BTreeMap::from([(64u32, 40i32)]))]);
        let capital_states = BTreeMap::from([("GER".to_string(), 64u32)]);
        let localisation = BTreeMap::new();
        let defines = sample_defines();
        let inputs = FurnitureLawInputs {
            province_ids: &ids,
            width,
            height,
            province_to_state: &province_to_state,
            state_owners: &state_owners,
            state_victory_points: &state_victory_points,
            capital_states: &capital_states,
            localisation: &localisation,
            vp_icon_defines: Some(&defines),
        };
        let markers = compute_furniture_markers(&inputs);
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].dot_tier, 2);
        assert_eq!(markers[0].kind, FurnitureMarkerKind::Capital);
    }

    #[test]
    fn dot_and_text_visibility_follow_shipped_defines() {
        let d = sample_defines();
        use FurnitureMarkerKind::{Capital, VictoryPoint};
        assert!(d.dot_visible(Capital, 1, 1099.9));
        assert!(!d.dot_visible(Capital, 1, 1100.0));
        assert!(d.text_visible(Capital, 0, 999.9, false));
        assert!(!d.text_visible(Capital, 0, 1000.0, false));
        assert!(!d.text_visible(Capital, 0, 0.0, true));
        assert!(d.dot_visible(VictoryPoint, 22, 999.9));
        assert!(!d.dot_visible(VictoryPoint, 22, 1000.0));
        assert!(
            d.dot_visible(VictoryPoint, 40, 999.9),
            "vp>22 remains visible just below the separate max cap"
        );
        assert!(!d.dot_visible(VictoryPoint, 40, 1000.0));
        assert!(d.dot_visible(VictoryPoint, 0, 99.9));
        assert!(!d.dot_visible(VictoryPoint, 0, 100.0));
        assert!(d.dot_visible(VictoryPoint, 11, 549.9));
        assert!(!d.dot_visible(VictoryPoint, 11, 550.0));
        assert!(d.text_visible(VictoryPoint, 0, 99.9, false));
        assert!(!d.text_visible(VictoryPoint, 0, 100.0, false));
        assert!(d.text_visible(VictoryPoint, 11, 449.9, false));
        assert!(!d.text_visible(VictoryPoint, 11, 450.0, false));
        assert!(d.text_visible(VictoryPoint, 22, 799.9, false));
        assert!(!d.text_visible(VictoryPoint, 22, 800.0, false));
        assert!(!d.text_visible(VictoryPoint, 22, 0.0, true));
    }

    #[test]
    fn vp_strip_frame_law_matches_measured_grid() {
        use FurnitureMarkerKind::{Capital, VictoryPoint};
        assert_eq!(
            vp_strip_frame_index(Capital, 0, VP_STRIP_COLOR_NEUTRAL_GRAY),
            4
        );
        assert_eq!(
            vp_strip_frame_index(Capital, 2, 0),
            0,
            "capital ignores tier"
        );
        assert_eq!(
            vp_strip_frame_index(VictoryPoint, 2, 4),
            9,
            "tier 2 = circle"
        );
        assert_eq!(
            vp_strip_frame_index(VictoryPoint, 1, 4),
            14,
            "tier 1 = pentagon"
        );
        assert_eq!(
            vp_strip_frame_index(VictoryPoint, 0, 4),
            19,
            "tier 0 = square"
        );
        assert_eq!(vp_strip_frame_index(VictoryPoint, 7, 0), 5);
        assert_eq!(vp_strip_frame_index(VictoryPoint, 0, 99), 19);
        for tier in 0..4u32 {
            for slot in 0..6u32 {
                assert!(vp_strip_frame_index(VictoryPoint, tier, slot) < VP_STRIP_FRAME_COUNT);
                assert!(vp_strip_frame_index(Capital, tier, slot) < VP_STRIP_FRAME_COUNT);
            }
        }
    }

    #[test]
    fn vp_icon_relation_column_matches_engine_priority() {
        let neutral = VpIconDiplomaticPairState::default();
        assert_eq!(
            vp_icon_relation_column(None, Some("GER"), neutral),
            VP_STRIP_COLOR_NEUTRAL_GRAY
        );
        assert_eq!(
            vp_icon_relation_column(Some("FRA"), None, neutral),
            VP_STRIP_COLOR_NEUTRAL_GRAY
        );
        assert_eq!(
            vp_icon_relation_column(Some(""), Some("GER"), neutral),
            VP_STRIP_COLOR_NEUTRAL_GRAY
        );
        assert_eq!(
            vp_icon_relation_column(Some("GER"), Some("GER"), neutral),
            VP_STRIP_COLOR_SELF_OR_FACTION
        );
        assert_eq!(
            vp_icon_relation_column(
                Some("GER"),
                Some("GER"),
                VpIconDiplomaticPairState {
                    same_faction: false,
                    at_war: true,
                },
            ),
            VP_STRIP_COLOR_SELF_OR_FACTION,
            "self/same-group path has priority over the enemy predicate"
        );
        assert_eq!(
            vp_icon_relation_column(
                Some("ITA"),
                Some("GER"),
                VpIconDiplomaticPairState {
                    same_faction: true,
                    at_war: true,
                },
            ),
            VP_STRIP_COLOR_SELF_OR_FACTION,
            "same faction/group is tested before at-war"
        );
        assert_eq!(
            vp_icon_relation_column(
                Some("SOV"),
                Some("GER"),
                VpIconDiplomaticPairState {
                    same_faction: false,
                    at_war: true,
                },
            ),
            VP_STRIP_COLOR_ENEMY_RED
        );
        assert_eq!(
            vp_icon_relation_column(Some("SWE"), Some("GER"), neutral),
            VP_STRIP_COLOR_NEUTRAL_GRAY
        );

        assert_eq!(VP_STRIP_COLOR_ENGINE_UNREACHED_1, 1);
        assert_eq!(VP_STRIP_COLOR_ENGINE_UNREACHED_3, 3);
        assert!(VP_STRIP_COLOR_ENEMY_RED < VP_STRIP_COLOR_SLOTS);
        assert!(VP_STRIP_COLOR_SELF_OR_FACTION < VP_STRIP_COLOR_SLOTS);
        assert!(VP_STRIP_COLOR_NEUTRAL_GRAY < VP_STRIP_COLOR_SLOTS);
    }

    #[test]
    fn vp_relation_column_composes_with_strip_frame_index() {
        use FurnitureMarkerKind::{Capital, VictoryPoint};
        assert_eq!(
            vp_strip_frame_index(Capital, 0, VP_STRIP_COLOR_ENEMY_RED),
            0
        );
        assert_eq!(
            vp_strip_frame_index(Capital, 0, VP_STRIP_COLOR_SELF_OR_FACTION),
            2
        );
        assert_eq!(
            vp_strip_frame_index(Capital, 0, VP_STRIP_COLOR_NEUTRAL_GRAY),
            4
        );
        assert_eq!(
            vp_strip_frame_index(VictoryPoint, 0, VP_STRIP_COLOR_ENEMY_RED),
            15
        );
        assert_eq!(
            vp_strip_frame_index(VictoryPoint, 2, VP_STRIP_COLOR_SELF_OR_FACTION),
            7
        );
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn gate_vanilla_vp_strip_grid_evidence() {
        let resolver = vanilla_sprite_resolver();
        let strip = load_vp_strip_rgba(&resolver).expect("vp strip decodes");
        assert_eq!(
            (strip.width, strip.height),
            (580, 27),
            "shipped strip geometry"
        );
        let frame_w = strip.width / VP_STRIP_FRAME_COUNT as usize;
        assert_eq!(frame_w, 29);
        let opaque = |frame: usize| -> usize {
            let x0 = frame * frame_w;
            let mut n = 0usize;
            for y in 0..strip.height {
                for x in x0..x0 + frame_w {
                    if strip.pixels[(y * strip.width + x) * 4 + 3] > 32 {
                        n += 1;
                    }
                }
            }
            n
        };
        let group_area: Vec<usize> = (0..4).map(|g| opaque(g * 5)).collect();
        for g in 0..4 {
            for c in 1..5 {
                assert_eq!(
                    opaque(g * 5 + c),
                    group_area[g],
                    "frame {} area differs from its shape group",
                    g * 5 + c
                );
            }
        }
        assert!(
            group_area[0] > group_area[1]
                && group_area[1] > group_area[2]
                && group_area[2] > group_area[3],
            "shape areas must order star > circle > pentagon > square, got {group_area:?}"
        );
        println!("gate: vp strip areas {group_area:?} (star/circle/pentagon/square)");
    }

    #[test]
    fn parse_lua_number_list_reads_shipped_shapes() {
        let src = "VICTORY_POINT_MAP_ICON_AFTER = {0, 9, 20}, -- comment\n";
        assert_eq!(
            parse_lua_number_list(src, "VICTORY_POINT_MAP_ICON_AFTER"),
            Some(vec![0.0, 9.0, 20.0])
        );
        assert_eq!(parse_lua_number_list(src, "VICTORY_POINT_MAP"), None);
        assert_eq!(parse_lua_number_list("KEY = 5,\n", "KEY"), None);
    }

    #[test]
    fn victory_point_loc_resolves_through_mod_override() {
        let root = temp_test_dir("map_furniture_loc_mod_override");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        write_text(
            &vanilla,
            "localisation/english/victory_points_l_english.yml",
            "\u{feff}l_english:\n VICTORY_POINTS_3838:0 \"Ajaccio\"\n VICTORY_POINTS_64:0 \"Berlin\"\n",
        );
        write_text(
            &mod_root,
            "localisation/english/victory_points_l_english.yml",
            "l_english:\n VICTORY_POINTS_3838:0 \"Ajaccio (modded)\"\n",
        );
        let resolver = GameFileResolver::with_mod_paths(vanilla, vec![mod_root]).expect("resolver");
        let table = localisation_table_from_resolver(&resolver, "english").expect("loc");
        assert_eq!(
            table.get("VICTORY_POINTS_3838").map(String::as_str),
            Some("Ajaccio (modded)")
        );
        assert_eq!(table.get("VICTORY_POINTS_64"), None);
        assert_eq!(resolver.stats().overridden_file_count, 1);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn capital_states_resolve_through_mod_override() {
        let root = temp_test_dir("map_furniture_capital_mod_override");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        write_text(
            &vanilla,
            "history/countries/GER - Germany.txt",
            "capital = 64 # Berlin\nowner = GER\n",
        );
        write_text(
            &mod_root,
            "history/countries/GER - Germany.txt",
            "capital = 999 # moved capital\n",
        );
        let resolver = GameFileResolver::with_mod_paths(vanilla, vec![mod_root]).expect("resolver");
        let capitals = capital_states_from_resolver(&resolver).expect("capitals");
        assert_eq!(capitals.get("GER").copied(), Some(999), "mod capital wins");
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn production_dlc_presentation_is_measured_exact_and_fail_closed() {
        let measured = session_metadata(Leaf::Measured(vec![
            "Together for Victory".to_string(),
            "Gotterdammerung".to_string(),
        ]));
        let presentation = ActiveDlcPresentation::from_session_metadata(&measured)
            .expect("measured DLC presentation");
        assert_eq!(
            presentation.display_names(),
            &[
                "Together for Victory".to_string(),
                "Gotterdammerung".to_string()
            ]
        );
        assert!(presentation.has_dlc("Together for Victory"));
        assert!(!presentation.has_dlc("together for victory"));

        let absent = session_metadata(Leaf::Absent);
        let error = ActiveDlcPresentation::from_session_metadata(&absent)
            .expect_err("absent DLC list must fail closed");
        assert!(error.to_string().contains("all-DLC fallback"));

        let unobtained = session_metadata(Leaf::Unobtained);
        let error = ActiveDlcPresentation::from_session_metadata(&unobtained)
            .expect_err("unobtained DLC list must fail closed");
        assert!(error.to_string().contains("all-DLC fallback"));

        let blank = session_metadata(Leaf::Measured(vec!["  ".to_string()]));
        assert!(ActiveDlcPresentation::from_session_metadata(&blank).is_err());
        let repeated = session_metadata(Leaf::Measured(vec![
            "Together for Victory".to_string(),
            "Together for Victory".to_string(),
        ]));
        let repeated = ActiveDlcPresentation::from_session_metadata(&repeated)
            .expect("the authenticated list contract permits repeated display names");
        assert_eq!(repeated.display_names().len(), 2);
        assert!(repeated.has_dlc("Together for Victory"));

        let mut unavailable_root = measured;
        unavailable_root.status = ViewStatus::DomainUnavailable;
        assert!(ActiveDlcPresentation::from_session_metadata(&unavailable_root).is_err());
    }

    #[test]
    fn production_name1b_uses_session_active_dlc_display_names() {
        let root = temp_test_dir("map_furniture_recording_dlc");
        write_text(
            &root,
            "history/countries/CAN - Canada.txt",
            "capital = 468\n\
             if = {\n\
             \tlimit = { NOT = { has_dlc = \"Together for Victory\" } }\n\
             \tset_cosmetic_tag = CAN_ALY\n\
             }\n\
             if = {\n\
             \tlimit = { has_dlc = \"Together for Victory\" }\n\
             \tset_cosmetic_tag = CAN_UK\n\
             }\n",
        );
        write_text(
            &root,
            "history/countries/RAJ - British Raj.txt",
            "capital = 439\nset_cosmetic_tag = RAJ_UK\n",
        );
        let resolver = GameFileResolver::vanilla(root.clone()).expect("fixture resolver");

        let with_dlc = session_metadata(Leaf::Measured(vec!["Together for Victory".to_string()]));
        let with_tags = cosmetic_tags_from_resolver(&resolver, &with_dlc)
            .expect("recording-owned with-DLC cosmetics");
        assert_eq!(with_tags.get("CAN").map(String::as_str), Some("CAN_UK"));
        assert_eq!(with_tags.get("RAJ").map(String::as_str), Some("RAJ_UK"));

        let without_dlc = session_metadata(Leaf::Measured(Vec::new()));
        let without_tags = cosmetic_tags_from_resolver(&resolver, &without_dlc)
            .expect("recording-owned no-DLC cosmetics");
        assert_eq!(without_tags.get("CAN").map(String::as_str), Some("CAN_ALY"));
        assert_eq!(without_tags.get("RAJ").map(String::as_str), Some("RAJ_UK"));

        let wrong_case = session_metadata(Leaf::Measured(vec!["together for victory".to_string()]));
        let wrong_case_tags = cosmetic_tags_from_resolver(&resolver, &wrong_case)
            .expect("exact display-name predicate");
        assert_eq!(
            wrong_case_tags.get("CAN").map(String::as_str),
            Some("CAN_ALY"),
            "NAME1b must not case-fold or infer DLC display names"
        );
        fs::remove_dir_all(root).ok();
    }


    fn vanilla_resolver() -> GameFileResolver {
        GameFileResolver::vanilla(replay_test_gates::require_vanilla_game_root())
            .expect("vanilla resolver")
    }

    /// The sprite path runs `require_complete_admitted_dir` over the whole
    /// `interface/*.gfx` namespace, so the admission has to list every one of
    /// them, not just the file the sprite resolves to.
    fn vanilla_sprite_resolver() -> GameFileResolver {
        let resolver = vanilla_resolver();
        let mut paths = resolver
            .audit_effective_dir_files("interface", Some("gfx"))
            .expect("interface gfx census")
            .into_iter()
            .collect::<BTreeSet<_>>();
        assert!(
            !paths.is_empty(),
            "the vanilla install must ship interface/*.gfx sprite catalogues"
        );
        for source in [
            VP_STRIP_SPRITE_TEXTURE,
            HOI_20BS_FNT_SOURCE,
            HOI_20BS_ATLAS_SOURCE,
        ] {
            paths.insert(source.to_string());
        }
        crate::test_support::admitted_vanilla_resolver(&resolver, &paths, "xmap21-furniture-gate").0
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn gate_vanilla_dlc_conditional_cosmetics_canada_congo() {
        let resolver = vanilla_resolver();
        let all_dlc =
            cosmetic_tags_from_resolver_with_dlc(&resolver, |_| true).expect("all-DLC cosmetics");
        assert_eq!(
            all_dlc.get("CAN").map(String::as_str),
            Some("CAN_UK"),
            "Canada with Together for Victory shows CAN_UK"
        );
        assert_eq!(
            all_dlc.get("COG").map(String::as_str),
            Some("COG_belgian_congo"),
            "Congo with Gotterdammerung shows COG_belgian_congo"
        );

        let no_dlc =
            cosmetic_tags_from_resolver_with_dlc(&resolver, |_| false).expect("no-DLC cosmetics");
        assert_eq!(
            no_dlc.get("CAN").map(String::as_str),
            Some("CAN_ALY"),
            "Canada without the DLC falls back to CAN_ALY"
        );
        assert_eq!(
            no_dlc.get("COG"),
            None,
            "Congo without the DLC has no cosmetic"
        );

        assert_eq!(all_dlc.get("RAJ").map(String::as_str), Some("RAJ_UK"));
        assert_eq!(no_dlc.get("RAJ").map(String::as_str), Some("RAJ_UK"));

        let mut differing: Vec<&str> = all_dlc
            .keys()
            .chain(no_dlc.keys())
            .filter(|tag| all_dlc.get(*tag) != no_dlc.get(*tag))
            .map(String::as_str)
            .collect();
        differing.sort_unstable();
        differing.dedup();
        assert_eq!(
            differing,
            vec!["BUL", "CAN", "COG", "XSM"],
            "DLC-conditional cosmetic set changed — a game update altered it"
        );
        assert_eq!(all_dlc.get("XSM"), None, "XSM has no cosmetic with all DLC");
        assert_eq!(
            no_dlc.get("XSM").map(String::as_str),
            Some("united_ma_clique_tag"),
            "XSM without No Compromise No Surrender takes the united-Ma cosmetic"
        );
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn gate_vanilla_province_centers_on_real_definition_csv() {
        let resolver = vanilla_resolver();
        let definition_csv = resolver
            .resolve_required("map/definition.csv", "definition.csv")
            .expect("definition.csv");
        let provinces_bmp = resolver
            .resolve_required("map/provinces.bmp", "provinces.bmp")
            .expect("provinces.bmp");
        let (definitions, color_to_id) =
            load_definition_csv(&definition_csv).expect("definition.csv rows");
        assert!(
            definitions.len() > 10_000,
            "vanilla definition.csv should have many rows, got {}",
            definitions.len()
        );
        let bmp = load_provinces_bmp(&provinces_bmp, &color_to_id).expect("provinces.bmp raster");
        assert_eq!((bmp.width, bmp.height), (5632, 2048), "vanilla map size");
        let mut vps = BTreeMap::new();
        vps.insert(1u32, BTreeMap::from([(3838u32, 1i32)]));
        let markers = compute_furniture_markers(&FurnitureLawInputs {
            province_ids: &bmp.ids,
            width: bmp.width,
            height: bmp.height,
            province_to_state: &BTreeMap::from([(3838u32, 1u32)]),
            state_owners: &BTreeMap::from([(1u32, "FRA".to_string())]),
            state_victory_points: &vps,
            capital_states: &BTreeMap::new(),
            localisation: &BTreeMap::new(),
            vp_icon_defines: None,
        });
        assert_eq!(markers.len(), 1, "province 3838 should be in the raster");
        let m = &markers[0];
        assert!(m.anchor_x >= 0.0 && (m.anchor_x as usize) < bmp.width);
        assert!(m.anchor_y >= 0.0 && (m.anchor_y as usize) < bmp.height);
        assert!(m.pixel_count > 0);
        println!(
            "gate: province 3838 centroid ({:.1},{:.1}) over {} px",
            m.anchor_x, m.anchor_y, m.pixel_count
        );
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn gate_vanilla_sprite_and_font_resolve() {
        let resolver = vanilla_sprite_resolver();
        let sprite = vp_strip_texture_path(&resolver)
            .expect("resolve vp strip")
            .expect("vp strip texture present");
        assert!(sprite.is_file(), "onmap_victorypoints_strip.dds present");
        let (font, atlas) = load_vp_label_font(&resolver).expect("hoi_20bs font");
        assert!(
            font.glyphs.len() > 200,
            "hoi_20bs should carry a full glyph set, got {}",
            font.glyphs.len()
        );
        assert_eq!((atlas.width, atlas.height), (512, 256), "hoi_20bs atlas");
        assert!(font.glyph(76).is_some());
        assert!(font.glyph(111).is_some());
        println!(
            "gate: hoi_20bs {} glyphs, atlas {}x{}, sprite {}",
            font.glyphs.len(),
            atlas.width,
            atlas.height,
            sprite.display()
        );
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn gate_vanilla_vp_icon_defines_match_shipped_values() {
        let resolver = vanilla_resolver();
        let defines = vp_icon_defines_from_resolver(&resolver).expect("vp icon defines");
        assert_eq!(defines.levels, 3);
        assert_eq!(defines.map_icon_after, vec![0, 9, 20]);
        assert_eq!(defines.text_cutoff, vec![150.0, 250.0, 500.0]);
        assert_eq!(defines.capital_icon_cutoff, 1100.0);
        println!("gate: vp icon defines {defines:?}");
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn gate_vanilla_full_furniture_layer() {
        let resolver = vanilla_resolver();
        let layer = furniture_layer_from_resolver(&resolver, "english").expect("furniture layer");
        assert_eq!((layer.width, layer.height), (5632, 2048));
        assert!(
            layer.markers.len() > 1_000,
            "vanilla has many VP markers, got {}",
            layer.markers.len()
        );
        assert!(layer.capital_count > 100, "capitals resolved");
        let capitals = layer
            .markers
            .iter()
            .filter(|m| m.kind == FurnitureMarkerKind::Capital)
            .count();
        assert!(capitals > 50, "many capital markers, got {capitals}");
        let berlin = layer.markers.iter().find(|m| m.province_id == 64);
        if let Some(b) = berlin {
            println!(
                "gate: province 64 '{}' kind={} owner={} vp={} tier={} at ({:.0},{:.0})",
                b.label,
                b.kind.as_str(),
                b.owner,
                b.vp_value,
                b.dot_tier,
                b.anchor_x,
                b.anchor_y
            );
        }
        println!(
            "gate: {} markers ({} capitals), {} loc keys, {} capitals resolved",
            layer.markers.len(),
            capitals,
            layer.localisation_key_count,
            layer.capital_count
        );
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn gate_vanilla_capital_tie_rule_is_bounded() {
        let resolver = vanilla_resolver();
        let data = furniture_static_data_from_resolver(&resolver, "english")
            .expect("furniture static data");
        let capital_state_ids: std::collections::BTreeSet<u32> =
            data.capital_states.values().copied().collect();
        let tie_state = |vps: &BTreeMap<u32, i32>| -> Option<(Vec<u32>, u32)> {
            let max_vp = vps.values().copied().max().unwrap_or(0);
            let mut tied: Vec<u32> = vps
                .iter()
                .filter(|(_, v)| **v == max_vp)
                .map(|(p, _)| *p)
                .collect();
            tied.sort_unstable();
            (tied.len() > 1).then(|| (tied.clone(), *tied.iter().min().unwrap()))
        };
        let mut census: Vec<(u32, u32)> = Vec::new();
        for (state_id, vps) in &data.state_victory_points {
            if capital_state_ids.contains(state_id) {
                if let Some((_, winner)) = tie_state(vps) {
                    census.push((*state_id, winner));
                }
            }
        }
        census.sort_unstable();
        assert_eq!(
            census,
            vec![
                (65, 514),
                (69, 3585),
                (328, 4572),
                (425, 4315),
                (459, 1145),
                (807, 3295),
            ],
            "vanilla capital-state VP ties changed — re-verify the bounded tie residual"
        );
        let layer = furniture_layer_from_resolver(&resolver, "english").expect("layer");
        let mut rendered_ties: Vec<(u32, u32)> = Vec::new();
        for (state_id, winner) in &census {
            let starred: Vec<u32> = layer
                .markers
                .iter()
                .filter(|m| m.state_id == *state_id && m.kind == FurnitureMarkerKind::Capital)
                .map(|m| m.province_id)
                .collect();
            if !starred.is_empty() {
                assert_eq!(
                    starred,
                    vec![*winner],
                    "capital state {state_id} stars must be exactly the lowest-id tied \
                     province {winner}"
                );
                rendered_ties.push((*state_id, *winner));
            }
        }
        println!(
            "gate: capital tie residual — {} vanilla capital-state ties in data, \
             {} rendered as 1936 stars: {:?}",
            census.len(),
            rendered_ties.len(),
            rendered_ties
        );
    }


    fn base_show_on_map() -> std::collections::BTreeSet<String> {
        std::collections::BTreeSet::from([
            AIR_BASE_BUILDING.to_string(),
            NAVAL_BASE_BUILDING.to_string(),
        ])
    }

    #[test]
    fn parse_state_building_levels_reads_air_and_naval() {
        let text = strip_comments(
            "state={ id=1 history={ owner = FRA\n buildings = {\n infrastructure = 2\n industrial_complex = 1\n air_base = 1\n 3838 = { naval_base = 3 }\n } } provinces={ 3838 } }",
        );
        let levels = parse_state_building_levels(&text).expect("buildings block");
        assert_eq!(levels.air_base, 1);
        assert_eq!(levels.naval_base.get(&3838).copied(), Some(3));
        assert!(parse_state_building_levels("state={ id=2 }").is_none());
        let bare = strip_comments("buildings = { infrastructure = 5 }");
        let bare = parse_state_building_levels(&bare).expect("bare buildings");
        assert_eq!(bare.air_base, 0);
        assert!(bare.naval_base.is_empty());
    }

    #[test]
    fn base_icon_frame_stays_inside_shipped_strip() {
        for level in 0..=15u32 {
            assert!(base_icon_frame(BaseIconKind::AirBase, level) < AIR_BASE_SPRITE_FRAMES);
            assert!(base_icon_frame(BaseIconKind::NavalBase, level) < NAVAL_BASE_SPRITE_FRAMES);
        }
        assert_eq!(base_icon_frame(BaseIconKind::AirBase, 3), 3);
        assert_eq!(
            base_icon_frame(BaseIconKind::AirBase, 10),
            6,
            "air clamps to 6"
        );
        assert_eq!(base_icon_frame(BaseIconKind::NavalBase, 10), 10);
    }

    #[test]
    fn building_locator_pixel_flips_world_z() {
        let (px, py) = building_locator_pixel([2948.38, 11.6, 1359.45], 2048);
        assert!((px - 2948.38).abs() < 1e-3);
        assert!((py - ((2048 - 1) as f32 - 1359.45)).abs() < 1e-3);
    }

    #[test]
    fn parse_base_icon_defines_reads_cutoffs() {
        let lua = "\
NDefines = { NGraphics = {\n\
\tAIRBASE_ICON_DISTANCE_CUTOFF = 900, -- At what distance air bases are hidden\n\
\tNAVALBASE_ICON_DISTANCE_CUTOFF = 900, -- 1300, -- naval bases\n\
} }\n";
        let d = parse_base_icon_defines(lua).expect("cutoffs");
        assert_eq!(d.airbase_cutoff, 900.0);
        assert_eq!(d.navalbase_cutoff, 900.0);
        assert!(d.visible(BaseIconKind::AirBase, 900.0));
        assert!(!d.visible(BaseIconKind::AirBase, 900.1));
        assert!(!d.visible(BaseIconKind::NavalBase, 901.0));
    }

    #[test]
    fn air_and_naval_base_icons_place_on_locators() {
        let width = 40;
        let height = 30;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 100, (10, 10, 20, 20));
        let province_to_state = BTreeMap::from([(100u32, 5u32)]);
        let state_owners = BTreeMap::from([(5u32, "FRA".to_string())]);
        let air_base_levels = BTreeMap::from([(5u32, 3u32)]);
        let naval_base_levels = BTreeMap::from([(100u32, 2u32)]);
        let air = MapBuildingPlacement {
            state_id: 5,
            building: AIR_BASE_BUILDING.to_string(),
            pos: [15.0, 0.0, (height - 1) as f32 - 15.0],
            rotation: 0.0,
            adjacent_sea_province: 0,
        };
        let naval = MapBuildingPlacement {
            state_id: 5,
            building: NAVAL_BASE_SPAWN_LOCATOR.to_string(),
            pos: [20.0, 0.0, (height - 1) as f32 - 15.0],
            rotation: 0.0,
            adjacent_sea_province: 900,
        };
        let show = base_show_on_map();
        let layer = compute_base_icon_markers(&BaseIconInputs {
            placements: &[air, naval],
            province_ids: &ids,
            width,
            height,
            province_to_state: &province_to_state,
            state_owners: &state_owners,
            air_base_levels: &air_base_levels,
            naval_base_levels: &naval_base_levels,
            show_on_map: &show,
        });
        assert_eq!(layer.air_base_count, 1);
        assert_eq!(layer.naval_base_count, 1);
        assert!(layer.naval_unattributed.is_empty());
        let a = layer
            .markers
            .iter()
            .find(|m| m.kind == BaseIconKind::AirBase)
            .unwrap();
        assert_eq!(
            (a.state_id, a.province_id, a.level, a.frame),
            (5, None, 3, 3)
        );
        assert!((a.anchor_x - 15.0).abs() < 0.01 && (a.anchor_y - 15.0).abs() < 0.01);
        let n = layer
            .markers
            .iter()
            .find(|m| m.kind == BaseIconKind::NavalBase)
            .unwrap();
        assert_eq!(
            (n.state_id, n.province_id, n.level, n.frame),
            (5, Some(100), 2, 2)
        );
        assert!((n.anchor_x - 20.0).abs() < 0.01 && (n.anchor_y - 15.0).abs() < 0.01);
        assert_eq!(n.owner, "FRA");
    }

    #[test]
    fn naval_base_without_locator_fails_closed() {
        let width = 40;
        let height = 30;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 100, (10, 10, 20, 20));
        let province_to_state = BTreeMap::from([(100u32, 5u32)]);
        let state_owners = BTreeMap::from([(5u32, "FRA".to_string())]);
        let naval_base_levels = BTreeMap::from([(100u32, 2u32)]);
        let show = base_show_on_map();
        let layer = compute_base_icon_markers(&BaseIconInputs {
            placements: &[],
            province_ids: &ids,
            width,
            height,
            province_to_state: &province_to_state,
            state_owners: &state_owners,
            air_base_levels: &BTreeMap::new(),
            naval_base_levels: &naval_base_levels,
            show_on_map: &show,
        });
        assert_eq!(layer.naval_base_count, 0);
        assert_eq!(layer.naval_unattributed, vec![100]);
    }

    #[test]
    fn base_icons_suppressed_when_not_show_on_map() {
        let width = 40;
        let height = 30;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 100, (10, 10, 20, 20));
        let air = MapBuildingPlacement {
            state_id: 5,
            building: AIR_BASE_BUILDING.to_string(),
            pos: [15.0, 0.0, (height - 1) as f32 - 15.0],
            rotation: 0.0,
            adjacent_sea_province: 0,
        };
        let layer = compute_base_icon_markers(&BaseIconInputs {
            placements: &[air],
            province_ids: &ids,
            width,
            height,
            province_to_state: &BTreeMap::from([(100u32, 5u32)]),
            state_owners: &BTreeMap::from([(5u32, "FRA".to_string())]),
            air_base_levels: &BTreeMap::from([(5u32, 3u32)]),
            naval_base_levels: &BTreeMap::new(),
            show_on_map: &std::collections::BTreeSet::new(),
        });
        assert!(layer.markers.is_empty());
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn gate_vanilla_base_icon_defines_match_shipped() {
        let resolver = vanilla_resolver();
        let d = base_icon_defines_from_resolver(&resolver).expect("base icon defines");
        assert_eq!(d.airbase_cutoff, 900.0, "AIRBASE_ICON_DISTANCE_CUTOFF");
        assert_eq!(d.navalbase_cutoff, 900.0, "NAVALBASE_ICON_DISTANCE_CUTOFF");
        println!("gate: base icon defines {d:?}");
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn gate_vanilla_base_icon_layer() {
        let resolver = vanilla_resolver();
        let layer = base_icon_layer_from_resolver(&resolver).expect("base icon layer");
        assert!(
            layer.air_base_count > 300,
            "expected many air bases, got {}",
            layer.air_base_count
        );
        assert!(
            layer.naval_base_count > 400,
            "expected many naval bases, got {}",
            layer.naval_base_count
        );
        for m in &layer.markers {
            assert!(m.frame < m.kind.sprite_frames(), "frame in strip");
            assert!(m.anchor_x >= 0.0 && m.anchor_y >= 0.0);
        }
        let total_naval = layer.naval_base_count + layer.naval_unattributed.len();
        let rate = layer.naval_base_count as f32 / total_naval as f32;
        assert!(
            rate > 0.95,
            "naval spawn attribution rate {rate:.3} ({} of {total_naval}); unattributed {:?}",
            layer.naval_base_count,
            layer.naval_unattributed
        );
        println!(
            "gate: FURN2 {} air + {} naval icons, naval attribution {:.3} ({} unattributed)",
            layer.air_base_count,
            layer.naval_base_count,
            rate,
            layer.naval_unattributed.len()
        );
    }

}
