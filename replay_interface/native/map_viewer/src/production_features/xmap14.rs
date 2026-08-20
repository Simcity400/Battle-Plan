
use crate::{
    combat_indicator_assets_from_resolver, parse_define_number, CombatIndicatorAssetManifest,
    FoundationFeatureContext, FoundationFeatureOutput, FoundationGap, GameFileResolver,
    ProductionMapFeatureAdapter, Result, Xmap13UnitIdentity,
};
use portable_content_manifest::{normalize_game_relative_path, UsedContentKey};
use portable_render_stream::{AssignedStage, FamilyDisposition, MapVisualFamily};
use replay_core::{CombatRow, Leaf, ValidatedContentAdmission, ViewStatus, WorldFrame};
use serde::{Deserialize, Serialize};
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet};

pub const XMAP14_INPUT_SCHEMA: &str = "hoi4.xmap14.combat-indicators.a729.v1";
pub const XMAP14_PREPARED_SCHEMA: &str = "hoi4.xmap14.combat-indicators-prepared.a729.v1";
pub const XMAP14_NATIVE_RECEIPT_SCHEMA: &str =
    "hoi4.xmap14.combat-indicators-native-receipt.a729.v1";
pub const XMAP14_ENGINE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const XMAP14_COMBAT_STAGE_NAME: &str = "xmap14.combat_indicators.execute";
pub const XMAP14_COMBAT_OPERATION_NAME: &str = "combat.indicators.draw";

const GRAPHICS_DEFINES: &str = "common/defines/00_graphics.lua";
const OWNED_GAPS: [&str; 3] = [
    "combat.land_active_epoch",
    "combat.land_terminal_detail",
    "combat.naval_air_epoch",
];

const REQUIRED_INTERFACE_SOURCES: [&str; 7] = [
    "interface/mapicons.gfx",
    "interface/mapicons.gui",
    "interface/landcombat.gfx",
    "interface/navalcombat.gfx",
    "interface/unitview.gfx",
    "interface/building_roster/naval_leader_module.gfx",
    "interface/core.gfx",
];

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Xmap14IndicatorIdentity {
    Land {
        combat_id: i64,
        generation: i64,
    },
    Naval {
        combat_id: i64,
        generation: i64,
    },
    NavalResult {
        result_id: i64,
        generation: i64,
        create_sequence: u64,
    },
}

impl Xmap14IndicatorIdentity {
    fn validate(&self) -> Result<()> {
        match self {
            Self::Land {
                combat_id,
                generation,
            }
            | Self::Naval {
                combat_id,
                generation,
            } if *combat_id <= 0 || *generation < 0 => {
                Err("XMAP14 active combat identity is invalid".into())
            }
            Self::NavalResult {
                result_id,
                generation,
                create_sequence,
            } if *result_id <= 0 || *generation < 0 || *create_sequence == 0 => {
                Err("XMAP14 naval-result identity is invalid".into())
            }
            _ => Ok(()),
        }
    }

    pub fn canonical_id(&self) -> String {
        match self {
            Self::Land {
                combat_id,
                generation,
            } => format!("land:{combat_id}:{generation}"),
            Self::Naval {
                combat_id,
                generation,
            } => format!("naval:{combat_id}:{generation}"),
            Self::NavalResult {
                result_id,
                generation,
                create_sequence,
            } => format!("naval_result:{result_id}:{generation}:{create_sequence}"),
        }
    }

    pub fn map_icon_bucket_ordinal(&self) -> u8 {
        match self {
            Self::Naval { .. } => 9,
            Self::NavalResult { .. } => 10,
            Self::Land { .. } => 11,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap14LandCarrier {
    pub identity: Xmap14IndicatorIdentity,
    pub location_province_id: u32,
    pub combat_base_point_f32_bits: [u32; 3],
    pub first_live_attacker_stack_point_f32_bits: [u32; 3],
    pub progress_fixed5: i64,
    pub attacker_comparison_fixed: i64,
    pub defender_comparison_fixed: i64,
    pub attacker_side_flags: u32,
    pub defender_side_flags: u32,
    pub side_flags_sha256: String,
    pub frame_selector: u8,
    pub attackers: Vec<Xmap13UnitIdentity>,
    pub defenders: Vec<Xmap13UnitIdentity>,
    pub active_phase: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap14NavalCategoryCounts {
    pub ordered_counts: [u32; 7],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap14NavalCarrier {
    pub identity: Xmap14IndicatorIdentity,
    pub location_province_id: u32,
    pub anchor_point_f32_bits: [u32; 3],
    pub progress_fixed5: i64,
    pub attackers: Vec<Xmap13UnitIdentity>,
    pub defenders: Vec<Xmap13UnitIdentity>,
    pub attacker_category_counts: Xmap14NavalCategoryCounts,
    pub defender_category_counts: Xmap14NavalCategoryCounts,
    pub category_counts_sha256: String,
    pub active_phase: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap14NavalResultMember {
    pub member_id: i64,
    pub frame_state_ready: bool,
    pub result_predicate: bool,
    pub first_side_countries: Vec<String>,
    pub second_side_countries: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Xmap14NavalResultSubtype {
    NavalBattle,
    PortStrike,
    NavalStrike,
    Convoy,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap14NavalResultCarrier {
    pub identity: Xmap14IndicatorIdentity,
    pub location_province_id: u32,
    pub alternate_anchor_branch: bool,
    pub anchor_point_f32_bits: [u32; 3],
    pub subtype: Xmap14NavalResultSubtype,
    pub members: Vec<Xmap14NavalResultMember>,
    pub retire_sequence: Option<u64>,
    pub lifecycle_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap14CombatEnvelope {
    pub schema: String,
    pub hour_index: i64,
    pub cadence: String,
    pub identity_order: String,
    pub capture_sequence: u64,
    pub producer_executable_sha256: String,
    pub producer_evidence_manifest_sha256: String,
    pub complete_land_census: bool,
    pub complete_naval_census: bool,
    pub complete_naval_result_census: bool,
    pub ordered_identity_sha256: String,
    pub census_sha256: String,
    pub land: Vec<Xmap14LandCarrier>,
    pub naval: Vec<Xmap14NavalCarrier>,
    pub naval_results: Vec<Xmap14NavalResultCarrier>,
    pub deterministic_sha256: String,
}

impl Xmap14CombatEnvelope {
    pub fn seal(mut self) -> Result<Self> {
        self.ordered_identity_sha256 = ordered_identity_sha256(&self);
        self.census_sha256 = census_sha256(&self)?;
        self.deterministic_sha256.clear();
        self.validate_body()?;
        self.deterministic_sha256 = Sha256::hex(&serde_json::to_vec(&self)?);
        self.validate_for_hour(self.hour_index)?;
        Ok(self)
    }

    pub fn validate_for_hour(&self, hour_index: i64) -> Result<()> {
        self.validate_body()?;
        if self.hour_index != hour_index {
            return Err("XMAP14 carrier belongs to a different replay hour".into());
        }
        validate_sha256(&self.deterministic_sha256, "XMAP14 carrier")?;
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        if self.deterministic_sha256 != Sha256::hex(&serde_json::to_vec(&unsigned)?) {
            return Err("XMAP14 carrier digest mismatch".into());
        }
        Ok(())
    }

    fn validate_body(&self) -> Result<()> {
        if self.schema != XMAP14_INPUT_SCHEMA
            || self.cadence != "every_game_hour_complete"
            || self.identity_order != "land_then_naval_then_retained_results_engine_order"
            || self.capture_sequence == 0
            || self.producer_executable_sha256 != XMAP14_ENGINE_SHA256
            || !self.complete_land_census
            || !self.complete_naval_census
            || !self.complete_naval_result_census
        {
            return Err("XMAP14 carrier header/completeness contract mismatch".into());
        }
        validate_sha256(
            &self.producer_evidence_manifest_sha256,
            "XMAP14 producer evidence",
        )?;
        validate_sha256(&self.ordered_identity_sha256, "XMAP14 identity order")?;
        validate_sha256(&self.census_sha256, "XMAP14 census")?;
        if self.ordered_identity_sha256 != ordered_identity_sha256(self)
            || self.census_sha256 != census_sha256(self)?
        {
            return Err("XMAP14 census/order digest mismatch".into());
        }

        let mut identities = BTreeSet::new();
        for row in &self.land {
            validate_land(row)?;
            if !identities.insert(row.identity.clone()) {
                return Err("XMAP14 indicator identity is duplicated".into());
            }
        }
        for row in &self.naval {
            validate_naval(row)?;
            if !identities.insert(row.identity.clone()) {
                return Err("XMAP14 indicator identity is duplicated".into());
            }
        }
        for row in &self.naval_results {
            validate_naval_result(row)?;
            if !identities.insert(row.identity.clone()) {
                return Err("XMAP14 indicator identity is duplicated".into());
            }
        }
        Ok(())
    }

    fn is_empty(&self) -> bool {
        self.land.is_empty() && self.naval.is_empty() && self.naval_results.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Xmap14ViewerSide {
    First,
    Second,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Xmap14LandTint {
    Winning,
    Balanced,
    Losing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Xmap14PreparedKind {
    Land,
    Naval,
    NavalResult,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PreparedXmap14SpriteDraw {
    pub texture_file: String,
    pub frame_index: u32,
    pub positions_f32_bits: [[u32; 2]; 4],
    pub uv_f32_bits: [[u32; 2]; 4],
    pub modulation_rgba_f32_bits: [u32; 4],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PreparedXmap14ProgressDraw {
    pub texture_file1: String,
    pub texture_file2: String,
    pub shader_file: String,
    pub shader_sha256: String,
    pub positions_f32_bits: [[u32; 2]; 4],
    pub uv_f32_bits: [[u32; 2]; 4],
    pub current_state_f32_bits: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PreparedXmap14TextDraw {
    pub font_name: String,
    pub font_language: String,
    pub text: String,
    pub box_min_f32_bits: [u32; 2],
    pub box_size_f32_bits: [u32; 2],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PreparedXmap14CounterDraw {
    pub grid_control_name: String,
    pub font_language: String,
    pub ordered_member_identities: Vec<Xmap13UnitIdentity>,
    pub member_count: u32,
    pub origin_f32_bits: [u32; 2],
    pub ordered_primitives: Vec<crate::Xmap13CounterPrimitive>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum PreparedXmap14Draw {
    Sprite(PreparedXmap14SpriteDraw),
    Progress(PreparedXmap14ProgressDraw),
    Text(PreparedXmap14TextDraw),
    Counter(PreparedXmap14CounterDraw),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedXmap14Indicator {
    pub identity: Xmap14IndicatorIdentity,
    pub kind: Xmap14PreparedKind,
    pub location_province_id: u32,
    pub world_anchor_f32_bits: [u32; 3],
    pub projected_screen_f32_bits: Option<[u32; 3]>,
    pub viewer_side: Xmap14ViewerSide,
    pub progress_fixed5: Option<i64>,
    pub progress_fraction_f32_bits: Option<u32>,
    pub land_tint: Option<Xmap14LandTint>,
    pub land_tint_rgba_f32_bits: Option<[u32; 4]>,
    pub bombardment_visible: bool,
    pub air_frame: Option<u8>,
    pub land_frame: Option<u8>,
    pub direction_radians_f32_bits: Option<u32>,
    pub naval_frame: Option<u8>,
    pub naval_large_progress: Option<bool>,
    pub naval_result_frame: Option<u8>,
    pub naval_result_subtype: Option<Xmap14NavalResultSubtype>,
    pub ordered_draws: Vec<PreparedXmap14Draw>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PreparedXmap14HitTarget {
    pub identity: Xmap14IndicatorIdentity,
    pub min_screen_f32_bits: [u32; 2],
    pub max_screen_f32_bits: [u32; 2],
    pub depth_f32_bits: u32,
}

impl PreparedXmap14HitTarget {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        let min = self.min_screen_f32_bits.map(f32::from_bits);
        let max = self.max_screen_f32_bits.map(f32::from_bits);
        x.is_finite() && y.is_finite() && x >= min[0] && x <= max[0] && y >= min[1] && y <= max[1]
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedXmap14Frame {
    pub schema: String,
    pub hour_index: i64,
    pub combat_indicator_assignment: AssignedStage,
    pub ordered_used_content: Vec<UsedContentKey>,
    pub indicators: Vec<PreparedXmap14Indicator>,
    pub hit_targets: Vec<PreparedXmap14HitTarget>,
    pub enumerator_witness_sha256: String,
    pub semantic_input_sha256: String,
    pub xmap13_census_sha256: String,
    pub xmap13_admission_sha256: String,
    pub deterministic_sha256: String,
}

impl PreparedXmap14Frame {
    pub fn ordered_identity_sha256(&self) -> String {
        prepared_identity_sha256(self)
    }

    pub fn hit_test(&self, screen_x: f32, screen_y: f32) -> Option<&Xmap14IndicatorIdentity> {
        self.hit_targets
            .iter()
            .filter(|row| row.contains(screen_x, screen_y))
            .min_by(|left, right| {
                right
                    .identity
                    .map_icon_bucket_ordinal()
                    .cmp(&left.identity.map_icon_bucket_ordinal())
                    .then_with(|| {
                        f32::from_bits(left.depth_f32_bits)
                            .total_cmp(&f32::from_bits(right.depth_f32_bits))
                    })
            })
            .map(|row| &row.identity)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP14_PREPARED_SCHEMA {
            return Err("XMAP14 prepared schema mismatch".into());
        }
        self.combat_indicator_assignment
            .validate()
            .map_err(|error| format!("XMAP14 combat assignment: {error}"))?;
        if self.combat_indicator_assignment.stage_name != XMAP14_COMBAT_STAGE_NAME
            || self.combat_indicator_assignment.operation_name != XMAP14_COMBAT_OPERATION_NAME
        {
            return Err("XMAP14 prepared assigned stage has the wrong identity".into());
        }
        validate_sha256(&self.semantic_input_sha256, "XMAP14 semantic input")?;
        validate_sha256(&self.enumerator_witness_sha256, "XMAP14 enumerator witness")?;
        validate_sha256(&self.xmap13_census_sha256, "XMAP13 census dependency")?;
        validate_sha256(&self.xmap13_admission_sha256, "XMAP13 admission dependency")?;
        validate_sha256(&self.deterministic_sha256, "XMAP14 prepared frame")?;
        if self.ordered_used_content.is_empty() {
            return Err("XMAP14 prepared effective-VFS closure is empty".into());
        }
        let mut admitted_paths = BTreeSet::new();
        for content in &self.ordered_used_content {
            content
                .validate()
                .map_err(|error| format!("XMAP14 prepared content key: {error}"))?;
            if !admitted_paths.insert(content.normalized_game_relative_path.as_str()) {
                return Err("XMAP14 prepared effective-VFS closure repeats a path".into());
            }
        }
        let mut ids = BTreeSet::new();
        if self
            .indicators
            .iter()
            .any(|row| !ids.insert(row.identity.clone()))
        {
            return Err("XMAP14 prepared indicator identity is duplicated".into());
        }
        if self.indicators.windows(2).any(|pair| {
            pair[0].identity.map_icon_bucket_ordinal() > pair[1].identity.map_icon_bucket_ordinal()
        }) {
            return Err("XMAP14 prepared indicators are not in exact engine bucket order".into());
        }
        for row in &self.indicators {
            if row.projected_screen_f32_bits.is_some() != !row.ordered_draws.is_empty() {
                return Err("XMAP14 projected indicator/draw coverage mismatch".into());
            }
            for draw in &row.ordered_draws {
                validate_prepared_draw(draw)?;
                for texture in prepared_draw_textures(draw) {
                    let normalized = normalize_game_relative_path(texture)
                        .map_err(|error| format!("XMAP14 prepared texture path: {error}"))?;
                    if !admitted_paths.contains(normalized.as_str()) {
                        return Err(format!(
                            "XMAP14 prepared draw references texture outside admitted closure: {normalized}"
                        )
                        .into());
                    }
                }
            }
        }
        let expected_hit_ids = self
            .indicators
            .iter()
            .filter(|row| row.projected_screen_f32_bits.is_some())
            .map(|row| &row.identity)
            .collect::<Vec<_>>();
        let actual_hit_ids = self
            .hit_targets
            .iter()
            .map(|target| &target.identity)
            .collect::<Vec<_>>();
        if actual_hit_ids != expected_hit_ids {
            return Err("XMAP14 projected indicator/hit-target coverage mismatch".into());
        }
        for target in &self.hit_targets {
            let min = target.min_screen_f32_bits.map(f32::from_bits);
            let max = target.max_screen_f32_bits.map(f32::from_bits);
            let depth = f32::from_bits(target.depth_f32_bits);
            if !min
                .into_iter()
                .chain(max)
                .chain([depth])
                .all(f32::is_finite)
                || min[0] > max[0]
                || min[1] > max[1]
            {
                return Err("XMAP14 prepared hit target is invalid".into());
            }
        }
        let expected = prepared_digest(self)?;
        if self.deterministic_sha256 != expected {
            return Err("XMAP14 prepared frame digest mismatch".into());
        }
        Ok(())
    }
}

fn prepared_draw_textures(draw: &PreparedXmap14Draw) -> Vec<&str> {
    match draw {
        PreparedXmap14Draw::Sprite(draw) => vec![draw.texture_file.as_str()],
        PreparedXmap14Draw::Progress(draw) => {
            vec![
                draw.texture_file1.as_str(),
                draw.texture_file2.as_str(),
                draw.shader_file.as_str(),
            ]
        }
        PreparedXmap14Draw::Text(_) => Vec::new(),
        PreparedXmap14Draw::Counter(draw) => draw
            .ordered_primitives
            .iter()
            .filter_map(|primitive| match primitive {
                crate::Xmap13CounterPrimitive::Sprite {
                    texture_content, ..
                } => Some(texture_content.normalized_game_relative_path.as_str()),
                _ => None,
            })
            .collect(),
    }
}

#[derive(Clone)]
struct Xmap14Content {
    assets: CombatIndicatorAssetManifest,
    ordered_used_content: Vec<UsedContentKey>,
    progress_localisation_template: String,
    localisation_language: String,
    progress_shaders: BTreeMap<String, ProgressShaderBinding>,
    land_cutoff: f32,
    naval_cutoff: f32,
    coarse_distance: f32,
    coarse_distance_strategic: f32,
    winning_color: [f32; 4],
    balanced_color: [f32; 4],
    losing_color: [f32; 4],
}

#[derive(Clone)]
struct ProgressShaderBinding {
    shader_file: String,
    shader_sha256: String,
}

const SUPPORTED_PROGRESS_SHADER_SOURCE: &str = r#"
Includes = { }
PixelShader = {
    Samplers = {
        TextureOne = {
            Index = 0
            MagFilter = "Point"
            MinFilter = "Point"
            MipFilter = "None"
            AddressU = "Wrap"
            AddressV = "Wrap"
        }
        TextureTwo = {
            Index = 1
            MagFilter = "Point"
            MinFilter = "Point"
            MipFilter = "None"
            AddressU = "Wrap"
            AddressV = "Wrap"
        }
    }
}
VertexStruct VS_INPUT {
    float4 vPosition : POSITION;
    float2 vTexCoord : TEXCOORD0;
};
VertexStruct VS_OUTPUT {
    float4 vPosition : PDX_POSITION;
    float2 vTexCoord0 : TEXCOORD0;
};
ConstantBuffer(0, 0) {
    float4x4 WorldViewProjectionMatrix;
    float4 vFirstColor;
    float4 vSecondColor;
    float CurrentState;
};
VertexShader = {
    MainCode VertexShader [[
        VS_OUTPUT main(const VS_INPUT v) {
            VS_OUTPUT Out;
            Out.vPosition = mul(WorldViewProjectionMatrix, v.vPosition);
            Out.vTexCoord0 = v.vTexCoord;
            Out.vTexCoord0.y = -Out.vTexCoord0.y;
            return Out;
        }
    ]]
}
PixelShader = {
    MainCode PixelColor [[
        float4 main(VS_OUTPUT v) : PDX_COLOR {
            if (v.vTexCoord0.x <= CurrentState)
                return vFirstColor;
            else
                return vSecondColor;
        }
    ]]
    MainCode PixelTexture [[
        float4 main(VS_OUTPUT v) : PDX_COLOR {
            if (v.vTexCoord0.x <= CurrentState)
                return tex2D(TextureOne, v.vTexCoord0.xy);
            else
                return tex2D(TextureTwo, v.vTexCoord0.xy);
        }
    ]]
}
BlendState BlendState {
    BlendEnable = yes
    SourceBlend = "SRC_ALPHA"
    DestBlend = "INV_SRC_ALPHA"
}
Effect Color {
    VertexShader = "VertexShader"
    PixelShader = "PixelColor"
}
Effect Texture {
    VertexShader = "VertexShader"
    PixelShader = "PixelTexture"
}
"#;

pub struct Xmap14Adapter {
    state: Xmap14State,
}

enum Xmap14State {
    Ready(Box<Xmap14Content>),
    /// The combat-indicator layer could not be built from THIS install's
    /// authored content, and says so in the reason.
    ///
    /// A DARK MAP LAYER IS EXCLUDED AND NAMED, NEVER A BLANK FRAME (owner
    /// ruling). Building this adapter reads the install's authored GFX, and any
    /// refusal in there used to propagate through the `?` in
    /// `production_foundation::product_feature_adapters_with_assigned_stage_schedule`
    /// and destroy the WHOLE adapter graph -- so one sprite cost the entire
    /// picture. Measured on t83: shipped vanilla authors
    /// `GFX_land_unit_combat_progressbar` (`interface/unitview.gfx:770`) with
    /// no `horizontal` key, as do 62 of the install's 105 `progressbartype`
    /// blocks, and the frame chain stopped there with no pixel, no BMP and no
    /// receipt.
    ///
    /// The reason travels verbatim, so nothing is hidden: a reader of the
    /// receipt can say which sprite, in which file, and why.
    Unavailable {
        reason: String,
    },
}

impl Xmap14Adapter {
    /// The adapter, ALWAYS -- excluding and naming the layer when this
    /// install's content cannot build it. See [`Xmap14State::Unavailable`].
    ///
    /// THERE IS NO FALLIBLE CONSTRUCTOR. A `new` that returned `Result` was
    /// written alongside this one and had no caller: it existed only so a test
    /// could pin the refusal, and the test never used it either, because
    /// building one needs a full `ValidatedContentAdmission`. Leaving it there
    /// left a way to put one sprite back in charge of the whole frame, so it is
    /// deleted (Codex review of batch 3). The refusal is not lost -- it is the
    /// `reason` this constructor stores, verbatim.
    pub fn new_or_unavailable(
        resolver: &GameFileResolver,
        admission: &ValidatedContentAdmission,
        localisation_language: &str,
    ) -> Self {
        match Self::content(resolver, admission, localisation_language) {
            Ok(content) => Self {
                state: Xmap14State::Ready(Box::new(content)),
            },
            Err(error) => Self {
                state: Xmap14State::Unavailable {
                    reason: error.to_string(),
                },
            },
        }
    }

    fn content(
        resolver: &GameFileResolver,
        admission: &ValidatedContentAdmission,
        localisation_language: &str,
    ) -> Result<Xmap14Content> {
        if !resolver.has_content_admission() {
            return Err("XMAP14 requires an admission-bound effective VFS".into());
        }
        resolver.require_complete_admitted_dir(
            "interface",
            Some("gfx"),
            true,
            "XMAP14 interface GFX namespace",
        )?;
        let language = canonical_localisation_language(localisation_language)?;
        resolver.require_complete_admitted_dir(
            &format!("localisation/{}", language.trim_start_matches("l_")),
            Some("yml"),
            false,
            "XMAP14 selected-language localisation namespace",
        )?;
        let assets = combat_indicator_assets_from_resolver(resolver)?;
        if !assets.unresolved_sprites.is_empty() {
            return Err("XMAP14 effective-VFS sprite closure is unresolved".into());
        }
        validate_combat_control_order(&assets)?;
        let graphics_defines = admission
            .require_used_content(GRAPHICS_DEFINES)
            .map_err(|error| format!("XMAP14 requires admitted graphics defines: {error}"))?
            .clone();
        let source = latin1(&resolver.read_admitted_used_content(&graphics_defines)?);
        let land_cutoff = parse_define_number(&source, "LAND_COMBAT_DISTANCE_CUTOFF")?;
        let naval_cutoff = parse_define_number(&source, "NAVAL_COMBAT_DISTANCE_CUTOFF")?;
        let coarse_distance =
            parse_define_number(&source, "MAP_ICONS_COARSE_COUNTRY_GROUPING_DISTANCE")?;
        let coarse_distance_strategic = parse_define_number(
            &source,
            "MAP_ICONS_COARSE_COUNTRY_GROUPING_DISTANCE_STRATEGIC",
        )?;
        let winning_color = parse_color4(&source, "LAND_COMBAT_WINNING_COLOR")?;
        let balanced_color = parse_color4(&source, "LAND_COMBAT_BALANCED_COLOR")?;
        let losing_color = parse_color4(&source, "LAND_COMBAT_LOSING_COLOR")?;
        if ![
            land_cutoff,
            naval_cutoff,
            coarse_distance,
            coarse_distance_strategic,
        ]
        .into_iter()
        .all(|value| value.is_finite() && value > 0.0)
        {
            return Err("XMAP14 graphics distance defines are invalid".into());
        }
        let (progress_localisation_template, localisation_paths) =
            combat_progress_localisation(resolver, admission, localisation_language)?;
        let localisation_language = canonical_localisation_language(localisation_language)?;
        let progress_shaders = progress_shader_bindings(resolver, admission, &assets)?;
        let ordered_used_content = admitted_content_closure(
            resolver,
            admission,
            &assets,
            graphics_defines.clone(),
            &localisation_paths,
            &progress_shaders,
            &localisation_language,
        )?;
        Ok(Xmap14Content {
            assets,
            ordered_used_content,
            progress_localisation_template,
            localisation_language,
            progress_shaders,
            land_cutoff,
            naval_cutoff,
            coarse_distance,
            coarse_distance_strategic,
            winning_color,
            balanced_color,
            losing_color,
        })
    }
}

impl ProductionMapFeatureAdapter for Xmap14Adapter {
    fn family(&self) -> MapVisualFamily {
        MapVisualFamily::CombatMarkers
    }

    fn prepare(&self, context: &FoundationFeatureContext<'_>) -> Result<FoundationFeatureOutput> {
        let consumed_gap_ids =
            require_owned_inputs(context.map_authority_gaps, context.map_authority_admitted)?;
        // The owned-input accounting happens FIRST and identically in both
        // states: a layer that is unavailable still consumed the same map
        // authority inputs, and a receipt that said otherwise would be a
        // different, quieter lie than the one this branch exists to prevent.
        let content = match &self.state {
            Xmap14State::Ready(content) => content.as_ref(),
            Xmap14State::Unavailable { reason } => {
                return Ok(unavailable_output_with_gap(
                    context.world_frame.hour_index(),
                    consumed_gap_ids,
                    "XMAP14.combat_indicator_content_unavailable",
                    reason,
                ));
            }
        };
        let Some(raw) = context.world_frame.xmap14_combat_indicator_carrier() else {
            return Ok(unavailable_output(
                context.world_frame.hour_index(),
                consumed_gap_ids,
                "recording has no complete exact XMAP14 combat-indicator carrier",
            ));
        };
        if raw.is_null() {
            return Ok(unavailable_output(
                context.world_frame.hour_index(),
                consumed_gap_ids,
                "XMAP14 combat-indicator carrier is explicitly unavailable at this hour",
            ));
        }
        let envelope: Xmap14CombatEnvelope = serde_json::from_value(raw.clone())
            .map_err(|error| format!("decode XMAP14 carrier: {error}"))?;
        envelope.validate_for_hour(context.world_frame.hour_index())?;
        let Some(units) = context.prepared_units else {
            return Ok(unavailable_output(
                context.world_frame.hour_index(),
                consumed_gap_ids,
                "earlier same-hour XMAP13 identity provider is unavailable",
            ));
        };
        let combat_indicator_assignment = context
            .require_assigned_stage(XMAP14_COMBAT_STAGE_NAME, XMAP14_COMBAT_OPERATION_NAME)?
            .clone();
        let prepared = content.prepare_exact(
            context.world_frame,
            context.viewer,
            &envelope,
            units,
            combat_indicator_assignment,
        )?;
        let registration = Sha256::hex(
            format!(
                "{XMAP14_PREPARED_SCHEMA}\0{XMAP14_ENGINE_SHA256}\0{}",
                envelope.producer_evidence_manifest_sha256
            )
            .as_bytes(),
        );
        let resources_sha256 = used_content_digest(&prepared.ordered_used_content)?;
        let target_sha256 = Sha256::hex(
            format!(
                "{}\0{}\0{}",
                context.viewer.viewport_width,
                context.viewer.viewport_height,
                context
                    .viewer
                    .camera
                    .as_ref()
                    .map(|camera| camera.deterministic_sha256.as_str())
                    .unwrap_or("<missing>")
            )
            .as_bytes(),
        );
        let disposition = if envelope.is_empty() {
            FamilyDisposition::AuthenticatedEmpty {
                producer_registration_sha256: registration,
                enumerator_witness_sha256: envelope.census_sha256.clone(),
            }
        } else {
            FamilyDisposition::Present {
                payload_schema: XMAP14_PREPARED_SCHEMA.into(),
                producer_registration_sha256: registration.clone(),
                payload_sha256: prepared.deterministic_sha256.clone(),
                ordered_operations: vec![prepared
                    .combat_indicator_assignment
                    .foundation_operation_identity(
                        registration,
                        resources_sha256,
                        target_sha256,
                    )?],
                ordered_used_content: prepared.ordered_used_content.clone(),
            }
        };
        Ok(FoundationFeatureOutput {
            disposition,
            prepared_terrain: None,
            prepared_political: None,
            prepared_thin_borders: None,
            prepared_rivers_straits: None,
            prepared_water: None,
            prepared_lighting: None,
            prepared_weather: None,
            prepared_effects: None,
            prepared_units: None,
            prepared_combat: Some(prepared),
            prepared_orders: None,
            prepared_rail_supply: None,
            prepared_map_modes: None,
            prepared_map_furniture: None,
            consumed_gap_ids,
        })
    }
}

impl Xmap14Content {
    fn prepare_exact(
        &self,
        frame: &WorldFrame,
        viewer: &crate::ViewerOwnedMapState,
        envelope: &Xmap14CombatEnvelope,
        units: &crate::PreparedXmap13Frame,
        combat_indicator_assignment: AssignedStage,
    ) -> Result<PreparedXmap14Frame> {
        units.validate()?;
        let provider = units.identity_provider();
        if provider.hour_index() != frame.hour_index() {
            return Err("XMAP14/XMAP13 dependency hour mismatch".into());
        }
        validate_typed_census(frame, envelope, provider)?;
        let camera = viewer
            .camera
            .as_ref()
            .ok_or("XMAP14 requires the XMAP03 viewer camera")?;
        camera.validate()?;
        let product_camera = camera.production_camera()?;
        let strategic = viewer.map_mode_id.contains("strategic");
        let camera_distance = camera.current_eye()[1];

        let mut indicators = Vec::with_capacity(
            envelope.land.len() + envelope.naval.len() + envelope.naval_results.len(),
        );
        for row in &envelope.land {
            let viewer_side = viewer_side_for_units(
                viewer.observer_country_id.as_deref(),
                &row.attackers,
                &row.defenders,
                provider,
            )?;
            let anchor = midpoint(
                row.combat_base_point_f32_bits,
                row.first_live_attacker_stack_point_f32_bits,
            );
            let projected = if camera_distance <= self.land_cutoff {
                project_map_icon(
                    &product_camera,
                    anchor,
                    viewer,
                    self.assets
                        .gui_layouts
                        .land_combat_mapicon
                        .size
                        .ok_or("XMAP14 land mapicon container has no size")?,
                )?
            } else {
                None
            };
            let progress = viewer_progress(row.progress_fixed5, viewer_side);
            let tint = land_tint(
                row.attacker_comparison_fixed,
                row.defender_comparison_fixed,
                viewer_side,
            )?;
            let flags = match viewer_side {
                Xmap14ViewerSide::First => row.attacker_side_flags,
                Xmap14ViewerSide::Second => row.defender_side_flags,
            };
            let air_frame = if flags & (1 << 15) != 0 {
                Some(3)
            } else if flags & (1 << 14) != 0 {
                Some(2)
            } else {
                None
            };
            let bombardment_visible =
                (row.attacker_side_flags | row.defender_side_flags) & ((1 << 11) | (1 << 12)) != 0;
            let direction = direction_radians(
                row.combat_base_point_f32_bits,
                row.first_live_attacker_stack_point_f32_bits,
            )?;
            let mut indicator = PreparedXmap14Indicator {
                identity: row.identity.clone(),
                kind: Xmap14PreparedKind::Land,
                location_province_id: row.location_province_id,
                world_anchor_f32_bits: anchor.map(f32::to_bits),
                projected_screen_f32_bits: projected,
                viewer_side,
                progress_fixed5: Some(progress),
                progress_fraction_f32_bits: Some((progress as f32 / 100_000.0).to_bits()),
                land_tint: Some(tint),
                land_tint_rgba_f32_bits: Some(
                    match tint {
                        Xmap14LandTint::Winning => self.winning_color,
                        Xmap14LandTint::Balanced => self.balanced_color,
                        Xmap14LandTint::Losing => self.losing_color,
                    }
                    .map(f32::to_bits),
                ),
                bombardment_visible,
                air_frame,
                land_frame: Some(if row.frame_selector == 0 { 1 } else { 2 }),
                direction_radians_f32_bits: Some(direction.to_bits()),
                naval_frame: None,
                naval_large_progress: None,
                naval_result_frame: None,
                naval_result_subtype: None,
                ordered_draws: Vec::new(),
            };
            indicator.ordered_draws = self.compose_indicator_draws(&indicator, None)?;
            indicators.push(indicator);
        }
        for row in &envelope.naval {
            let typed = typed_naval_combat(frame, &row.identity)?;
            let viewer_side = viewer_side_for_naval(viewer.observer_country_id.as_deref(), typed)?;
            let anchor = point(row.anchor_point_f32_bits)?;
            let projected = if camera_distance <= self.naval_cutoff {
                project_map_icon(
                    &product_camera,
                    anchor,
                    viewer,
                    self.assets
                        .gui_layouts
                        .naval_combat_mapicon
                        .size
                        .ok_or("XMAP14 naval mapicon container has no size")?,
                )?
            } else {
                None
            };
            let threshold = if strategic {
                self.coarse_distance_strategic
            } else {
                self.coarse_distance
            };
            let mut indicator = PreparedXmap14Indicator {
                identity: row.identity.clone(),
                kind: Xmap14PreparedKind::Naval,
                location_province_id: row.location_province_id,
                world_anchor_f32_bits: row.anchor_point_f32_bits,
                projected_screen_f32_bits: projected,
                viewer_side,
                progress_fixed5: Some(viewer_progress(row.progress_fixed5, viewer_side)),
                progress_fraction_f32_bits: Some(
                    (viewer_progress(row.progress_fixed5, viewer_side) as f32 / 100_000.0)
                        .to_bits(),
                ),
                land_tint: None,
                land_tint_rgba_f32_bits: None,
                bombardment_visible: false,
                air_frame: None,
                land_frame: None,
                direction_radians_f32_bits: None,
                naval_frame: Some(
                    u8::from(naval_convoy_frame(
                        &row.attacker_category_counts.ordered_counts,
                        &row.defender_category_counts.ordered_counts,
                    )) + 1,
                ),
                naval_large_progress: Some(camera_distance <= threshold),
                naval_result_frame: None,
                naval_result_subtype: None,
                ordered_draws: Vec::new(),
            };
            indicator.ordered_draws = self.compose_indicator_draws(
                &indicator,
                Some((row, provider, viewer.observer_country_id.as_deref(), frame)),
            )?;
            indicators.push(indicator);
        }
        for row in &envelope.naval_results {
            let anchor = point(row.anchor_point_f32_bits)?;
            let projected = if camera_distance <= self.naval_cutoff {
                project_map_icon(
                    &product_camera,
                    anchor,
                    viewer,
                    self.assets
                        .gui_layouts
                        .naval_combat_results_mapicon
                        .size
                        .ok_or("XMAP14 naval-result mapicon container has no size")?,
                )?
            } else {
                None
            };
            let mut indicator = PreparedXmap14Indicator {
                identity: row.identity.clone(),
                kind: Xmap14PreparedKind::NavalResult,
                location_province_id: row.location_province_id,
                world_anchor_f32_bits: row.anchor_point_f32_bits,
                projected_screen_f32_bits: projected,
                viewer_side: Xmap14ViewerSide::First,
                progress_fixed5: None,
                progress_fraction_f32_bits: None,
                land_tint: None,
                land_tint_rgba_f32_bits: None,
                bombardment_visible: false,
                air_frame: None,
                land_frame: None,
                direction_radians_f32_bits: None,
                naval_frame: None,
                naval_large_progress: None,
                naval_result_frame: Some(naval_result_frame(
                    row,
                    viewer.observer_country_id.as_deref(),
                )?),
                naval_result_subtype: Some(row.subtype.clone()),
                ordered_draws: Vec::new(),
            };
            indicator.ordered_draws = self.compose_indicator_draws(&indicator, None)?;
            indicators.push(indicator);
        }

        indicators.sort_by_key(|row| row.identity.map_icon_bucket_ordinal());

        let mut ordered_used_content = self.ordered_used_content.clone();
        let mut admitted_paths = ordered_used_content
            .iter()
            .map(|content| content.normalized_game_relative_path.clone())
            .collect::<BTreeSet<_>>();
        for identity in envelope
            .naval
            .iter()
            .flat_map(|row| row.attackers.iter().chain(row.defenders.iter()))
        {
            let counter = provider
                .lookup(identity)
                .and_then(|unit| unit.counter.as_ref())
                .ok_or("XMAP14 naval grid participant has no XMAP13 counter closure")?;
            for content in &counter.ordered_interface_content {
                if admitted_paths.insert(content.normalized_game_relative_path.clone()) {
                    ordered_used_content.push(content.clone());
                }
            }
        }
        let mut prepared = PreparedXmap14Frame {
            schema: XMAP14_PREPARED_SCHEMA.into(),
            hour_index: frame.hour_index(),
            combat_indicator_assignment,
            ordered_used_content,
            indicators,
            hit_targets: Vec::new(),
            enumerator_witness_sha256: envelope.census_sha256.clone(),
            semantic_input_sha256: envelope.deterministic_sha256.clone(),
            xmap13_census_sha256: provider.census_sha256().into(),
            xmap13_admission_sha256: provider.admission_sha256().into(),
            deterministic_sha256: String::new(),
        };
        prepared.hit_targets = prepared
            .indicators
            .iter()
            .filter_map(|indicator| self.hit_target(indicator).transpose())
            .collect::<Result<Vec<_>>>()?;
        prepared.deterministic_sha256 = prepared_digest(&prepared)?;
        prepared.validate()?;
        Ok(prepared)
    }

    fn compose_indicator_draws(
        &self,
        row: &PreparedXmap14Indicator,
        naval_source: Option<(
            &Xmap14NavalCarrier,
            &crate::Xmap13IdentityProvider,
            Option<&str>,
            &WorldFrame,
        )>,
    ) -> Result<Vec<PreparedXmap14Draw>> {
        let Some(screen_bits) = row.projected_screen_f32_bits else {
            return Ok(Vec::new());
        };
        let screen = [
            f32::from_bits(screen_bits[0]),
            f32::from_bits(screen_bits[1]),
        ];
        let white = [1.0f32.to_bits(); 4];
        let tint = row.land_tint_rgba_f32_bits.unwrap_or(white);
        let mut draws = Vec::new();
        match row.kind {
            Xmap14PreparedKind::Land => {
                let layout = &self.assets.gui_layouts.land_combat_mapicon;
                let origin = aligned_container_origin(screen, required_size(layout)?)?;
                let direction_control = required_control(layout, "battle_indicator_direction")?;
                draws.push(PreparedXmap14Draw::Sprite(sprite_draw(
                    land_sprite_for_control(&self.assets, direction_control)?,
                    0,
                    direction_control,
                    origin,
                    tint,
                    row.direction_radians_f32_bits.map(f32::from_bits),
                )?));
                let background_control = required_control(layout, "bg_btn")?;
                draws.push(PreparedXmap14Draw::Sprite(sprite_draw(
                    land_sprite_for_control(&self.assets, background_control)?,
                    0,
                    background_control,
                    origin,
                    tint,
                    None,
                )?));
                if row.air_frame.is_none() {
                    let frame = row
                        .land_frame
                        .ok_or("XMAP14 land icon omitted its ordinary frame")?;
                    let control = required_control(layout, "mapicon_land_combat_frame")?;
                    draws.push(PreparedXmap14Draw::Sprite(sprite_draw(
                        land_sprite_for_control(&self.assets, control)?,
                        one_based_frame_index(frame)?,
                        control,
                        origin,
                        tint,
                        None,
                    )?));
                }
                if row.bombardment_visible {
                    let control = required_control(layout, "mapicon_land_combat_bombardment")?;
                    draws.push(PreparedXmap14Draw::Sprite(sprite_draw(
                        land_sprite_for_control(&self.assets, control)?,
                        0,
                        control,
                        origin,
                        tint,
                        None,
                    )?));
                }
                if let Some(frame) = row.air_frame {
                    let control = required_control(layout, "air_superiority_icon")?;
                    draws.push(PreparedXmap14Draw::Sprite(sprite_draw(
                        land_sprite_for_control(&self.assets, control)?,
                        one_based_frame_index(frame)?,
                        control,
                        origin,
                        tint,
                        None,
                    )?));
                } else {
                    let control = required_control(layout, "combat_progress")?;
                    let width = control
                        .max_width
                        .ok_or("XMAP14 combat_progress has no FILE maxWidth")?;
                    let height = control
                        .max_height
                        .ok_or("XMAP14 combat_progress has no FILE maxHeight")?;
                    let progress = row
                        .progress_fixed5
                        .ok_or("XMAP14 land progress text has no fixed5 value")?;
                    let min = control_anchor(control, origin)?;
                    draws.push(PreparedXmap14Draw::Text(PreparedXmap14TextDraw {
                        font_name: control
                            .font
                            .clone()
                            .ok_or("XMAP14 combat_progress has no FILE font")?,
                        font_language: self.localisation_language.clone(),
                        text: format_combat_progress(
                            &self.progress_localisation_template,
                            progress,
                        )?,
                        box_min_f32_bits: min.map(f32::to_bits),
                        box_size_f32_bits: [(width as f32).to_bits(), (height as f32).to_bits()],
                    }));
                }
            }
            Xmap14PreparedKind::Naval => {
                let layout = &self.assets.gui_layouts.naval_combat_mapicon;
                let origin = aligned_container_origin(screen, required_size(layout)?)?;
                let frame = row
                    .naval_frame
                    .ok_or("XMAP14 naval icon omitted its active frame")?;
                let background_control = required_control(layout, "bg_btn")?;
                draws.push(PreparedXmap14Draw::Sprite(sprite_draw(
                    naval_sprite_for_control(&self.assets, background_control)?,
                    one_based_frame_index(frame)?,
                    background_control,
                    origin,
                    white,
                    None,
                )?));
                let frame_control = required_control(layout, "frame_btn")?;
                draws.push(PreparedXmap14Draw::Sprite(sprite_draw(
                    naval_sprite_for_control(&self.assets, frame_control)?,
                    0,
                    frame_control,
                    origin,
                    white,
                    None,
                )?));
                let (source, provider, viewer_country, frame) = naval_source
                    .ok_or("XMAP14 naval indicator lost its exact participant source")?;
                let large = row
                    .naval_large_progress
                    .ok_or("XMAP14 naval icon omitted its progress variant")?;
                if large {
                    draws.extend(naval_grid_draws(
                        source,
                        provider,
                        viewer_country,
                        frame,
                        &self.assets,
                        &self.localisation_language,
                        required_control(layout, "attacker_adds_grid")?,
                        origin,
                        true,
                        true,
                    )?);
                    draws.extend(naval_grid_draws(
                        source,
                        provider,
                        viewer_country,
                        frame,
                        &self.assets,
                        &self.localisation_language,
                        required_control(layout, "defender_adds_grid")?,
                        origin,
                        false,
                        true,
                    )?);
                }
                let control_name = if large { "progress" } else { "progress_small" };
                let control = required_control(layout, control_name)?;
                let bar_name = required_control_alias(control)?;
                let bar = self
                    .assets
                    .progress_bars
                    .get(bar_name)
                    .ok_or_else(|| format!("XMAP14 effective VFS omitted {bar_name}"))?;
                let shader = self
                    .progress_shaders
                    .get(bar_name)
                    .ok_or_else(|| format!("XMAP14 progress shader closure omitted {bar_name}"))?;
                let size = bar
                    .size
                    .ok_or_else(|| format!("XMAP14 progress bar {bar_name} has no FILE size"))?;
                let min = control_rect_min(control, origin, size)?;
                draws.push(PreparedXmap14Draw::Progress(PreparedXmap14ProgressDraw {
                    texture_file1: bar.texture_file1.clone(),
                    texture_file2: bar.texture_file2.clone(),
                    shader_file: shader.shader_file.clone(),
                    shader_sha256: shader.shader_sha256.clone(),
                    positions_f32_bits: axis_aligned_strip(min, size)?
                        .map(|value| value.map(f32::to_bits)),
                    uv_f32_bits: full_uv_strip().map(|value| value.map(f32::to_bits)),
                    current_state_f32_bits: row
                        .progress_fraction_f32_bits
                        .ok_or("XMAP14 naval progress has no fraction")?,
                }));
                if large {
                    draws.extend(naval_grid_draws(
                        source,
                        provider,
                        viewer_country,
                        frame,
                        &self.assets,
                        &self.localisation_language,
                        required_control(layout, "attacker_grid")?,
                        origin,
                        true,
                        false,
                    )?);
                    draws.extend(naval_grid_draws(
                        source,
                        provider,
                        viewer_country,
                        frame,
                        &self.assets,
                        &self.localisation_language,
                        required_control(layout, "defender_grid")?,
                        origin,
                        false,
                        false,
                    )?);
                }
            }
            Xmap14PreparedKind::NavalResult => {
                let layout = &self.assets.gui_layouts.naval_combat_results_mapicon;
                let origin = aligned_container_origin(screen, required_size(layout)?)?;
                let frame = row
                    .naval_result_frame
                    .ok_or("XMAP14 naval-result icon omitted its frame")?;
                let control = required_control(layout, "bg_btn")?;
                draws.push(PreparedXmap14Draw::Sprite(sprite_draw(
                    naval_sprite_for_control(&self.assets, control)?,
                    one_based_frame_index(frame)?,
                    control,
                    origin,
                    white,
                    None,
                )?));
            }
        }
        Ok(draws)
    }

    fn hit_target(
        &self,
        indicator: &PreparedXmap14Indicator,
    ) -> Result<Option<PreparedXmap14HitTarget>> {
        let Some(projected) = indicator.projected_screen_f32_bits else {
            return Ok(None);
        };
        let screen = projected.map(f32::from_bits);
        let size = match indicator.kind {
            Xmap14PreparedKind::Land => {
                required_size(&self.assets.gui_layouts.land_combat_mapicon)?
            }
            Xmap14PreparedKind::Naval => {
                required_size(&self.assets.gui_layouts.naval_combat_mapicon)?
            }
            Xmap14PreparedKind::NavalResult => {
                required_size(&self.assets.gui_layouts.naval_combat_results_mapicon)?
            }
        };
        let min = aligned_container_origin([screen[0], screen[1]], size)?;
        let max = [min[0] + size.width as f32, min[1] + size.height as f32];
        Ok(Some(PreparedXmap14HitTarget {
            identity: indicator.identity.clone(),
            min_screen_f32_bits: min.map(f32::to_bits),
            max_screen_f32_bits: max.map(f32::to_bits),
            depth_f32_bits: screen[2].to_bits(),
        }))
    }
}

fn validate_combat_control_order(assets: &CombatIndicatorAssetManifest) -> Result<()> {
    for (layout, expected) in [
        (
            &assets.gui_layouts.land_combat_mapicon,
            &[
                "battle_indicator_direction",
                "bg_btn",
                "mapicon_land_combat_frame",
                "mapicon_land_combat_bombardment",
                "combat_progress",
                "air_superiority_icon",
            ][..],
        ),
        (
            &assets.gui_layouts.naval_combat_mapicon,
            &[
                "bg_btn",
                "frame_btn",
                "attacker_adds_grid",
                "defender_adds_grid",
                "progress",
                "progress_small",
                "attacker_grid",
                "defender_grid",
            ][..],
        ),
        (
            &assets.gui_layouts.naval_combat_results_mapicon,
            &["bg_btn"][..],
        ),
        (
            &assets.gui_layouts.tiny_unit_counter,
            &[
                "background",
                "icon",
                "count",
                "spotting_bar_container",
                "spotting_bar_bg",
                "spotting_bar_fg_container",
                "spotting_bar_fg",
            ][..],
        ),
    ] {
        let actual = layout
            .controls
            .iter()
            .map(|control| control.name.as_str())
            .collect::<Vec<_>>();
        if actual != expected {
            return Err(format!(
                "XMAP14 effective GUI container {} has an unsupported painter/control order",
                layout.name
            )
            .into());
        }
    }
    let land_progress =
        required_control(&assets.gui_layouts.land_combat_mapicon, "combat_progress")?;
    if !land_progress
        .format
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case("center"))
    {
        return Err("XMAP14 effective GUI combat_progress must retain center text format".into());
    }
    Ok(())
}

fn naval_grid_draws(
    source: &Xmap14NavalCarrier,
    provider: &crate::Xmap13IdentityProvider,
    viewer_country: Option<&str>,
    frame: &WorldFrame,
    assets: &CombatIndicatorAssetManifest,
    localisation_language: &str,
    control: &crate::GuiControlLayout,
    container_origin: [f32; 2],
    attacker: bool,
    aggregate: bool,
) -> Result<Vec<PreparedXmap14Draw>> {
    let identities = if attacker {
        &source.attackers
    } else {
        &source.defenders
    };
    if identities.is_empty() {
        return Ok(Vec::new());
    }
    let slot = control
        .slot_size
        .ok_or_else(|| format!("XMAP14 naval grid {} has no FILE slot size", control.name))?;
    if slot.width <= 0 || slot.height <= 0 {
        return Err(format!(
            "XMAP14 naval grid {} has an invalid slot size",
            control.name
        )
        .into());
    }
    let base = control_anchor(control, container_origin)?;
    struct GridRow<'a> {
        allegiance: crate::Xmap13CounterAllegiance,
        counter_item_ordinal: u32,
        counter_item_member_ordinal: u32,
        member_ordinal: u32,
        counter_item_count: u32,
        counter_item_member_count: u32,
        counter_item_id: String,
        identity: Xmap13UnitIdentity,
        presentation: &'a crate::Xmap13CounterPresentation,
    }
    let mut rows = identities
        .iter()
        .map(|identity| {
            let unit = provider
                .lookup(identity)
                .ok_or("XMAP14 naval grid participant disappeared from XMAP13")?;
            let counter = unit
                .counter
                .as_ref()
                .ok_or("XMAP14 naval grid participant has no counter presentation")?;
            let allegiance = naval_counter_allegiance(frame, viewer_country, &unit.owner_country)?;
            let mut memberships = counter
                .ordered_group_memberships
                .iter()
                .filter(|membership| {
                    membership.grouping_tier == 0 && membership.allegiance == allegiance
                });
            let membership = memberships
                .next()
                .ok_or("XMAP14 naval grid participant lacks a tier-zero viewer presentation")?;
            if memberships.next().is_some() {
                return Err("XMAP14 naval grid tier-zero membership is ambiguous".into());
            }
            let presentation = counter
                .ordered_presentations
                .iter()
                .find(|presentation| {
                    presentation.grouping_tier == 0
                        && presentation.counter_item_id == membership.counter_item_id
                        && presentation.allegiance == allegiance
                        && !presentation.selected
                })
                .ok_or("XMAP14 naval grid participant lacks its unselected tier-zero primitives")?;
            if !aggregate {
                validate_counter_presentation_fonts(assets, presentation)?;
            }
            if membership.counter_item_member_count == 0 || membership.counter_item_count == 0 {
                return Err("XMAP14 naval grid membership has an empty counter-item census".into());
            }
            Ok(GridRow {
                allegiance,
                counter_item_ordinal: membership.counter_item_ordinal,
                counter_item_member_ordinal: membership.counter_item_member_ordinal,
                member_ordinal: membership.member_ordinal,
                counter_item_count: membership.counter_item_count,
                counter_item_member_count: membership.counter_item_member_count,
                counter_item_id: membership.counter_item_id.clone(),
                identity: identity.clone(),
                presentation,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    if aggregate {
        let representative = rows
            .first()
            .ok_or("XMAP14 aggregate naval grid lost its representative")?
            .presentation;
        let tint = counter_backing_modulation(representative)?;
        return tiny_counter_draws(
            assets,
            localisation_language,
            base,
            u32::try_from(rows.len())?,
            tint,
        );
    }
    let mut allegiance_seen = BTreeSet::new();
    let allegiance_order = rows
        .iter()
        .filter_map(|row| {
            allegiance_seen
                .insert(row.allegiance)
                .then_some(row.allegiance)
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| {
        (
            row.allegiance,
            row.counter_item_ordinal,
            row.counter_item_member_ordinal,
            row.member_ordinal,
        )
    });
    let mut declared_item_counts = BTreeMap::<crate::Xmap13CounterAllegiance, u32>::new();
    let mut grouped = BTreeMap::<
        (crate::Xmap13CounterAllegiance, u32),
        (
            String,
            u32,
            Vec<(u32, u32, Xmap13UnitIdentity)>,
            &crate::Xmap13CounterPresentation,
        ),
    >::new();
    for row in rows {
        let declared = declared_item_counts
            .entry(row.allegiance)
            .or_insert(row.counter_item_count);
        if *declared != row.counter_item_count {
            return Err("XMAP14 naval grid allegiance counter-item census is inconsistent".into());
        }
        let entry = grouped
            .entry((row.allegiance, row.counter_item_ordinal))
            .or_insert_with(|| {
                (
                    row.counter_item_id.clone(),
                    row.counter_item_member_count,
                    Vec::new(),
                    row.presentation,
                )
            });
        if entry.0 != row.counter_item_id
            || entry.1 != row.counter_item_member_count
            || entry.3.counter_item_id != row.presentation.counter_item_id
            || entry.3.allegiance != row.allegiance
            || entry.3.ordered_primitives != row.presentation.ordered_primitives
        {
            return Err("XMAP14 naval grid members disagree on their grouped presentation".into());
        }
        entry.2.push((
            row.counter_item_member_ordinal,
            row.member_ordinal,
            row.identity,
        ));
    }
    let mut draws = Vec::new();
    let mut slot_ordinal = 0u32;
    for allegiance in allegiance_order {
        let declared_item_count = *declared_item_counts
            .get(&allegiance)
            .ok_or("XMAP14 naval grid lost an allegiance census")?;
        if grouped
            .keys()
            .filter(|(candidate, _)| *candidate == allegiance)
            .count()
            != declared_item_count as usize
            || (0..declared_item_count).any(|ordinal| !grouped.contains_key(&(allegiance, ordinal)))
        {
            return Err(
                "XMAP14 naval grid counter-item ordinals are incomplete within an allegiance group"
                    .into(),
            );
        }
        for ordinal in 0..declared_item_count {
            let (_, expected_members, mut identities, presentation) = grouped
                .remove(&(allegiance, ordinal))
                .ok_or("XMAP14 naval grid lost an allegiance counter item")?;
            identities.sort_by_key(|member| (member.0, member.1));
            if identities
                .windows(2)
                .any(|pair| (pair[0].0, pair[0].1) >= (pair[1].0, pair[1].1))
                || identities.len() != expected_members as usize
            {
                return Err("XMAP14 naval grid group member census/order is invalid".into());
            }
            let origin = [base[0], base[1] + slot_ordinal as f32 * slot.height as f32];
            slot_ordinal = slot_ordinal
                .checked_add(1)
                .ok_or("XMAP14 naval grid slot ordinal overflow")?;
            draws.push(PreparedXmap14Draw::Counter(PreparedXmap14CounterDraw {
                grid_control_name: control.name.clone(),
                font_language: canonical_localisation_language(localisation_language)?,
                member_count: u32::try_from(identities.len())?,
                ordered_member_identities: identities.into_iter().map(|member| member.2).collect(),
                origin_f32_bits: origin.map(f32::to_bits),
                ordered_primitives: presentation.ordered_primitives.clone(),
            }));
        }
    }
    if !grouped.is_empty() {
        return Err("XMAP14 naval grid retained an unconsumed allegiance group".into());
    }
    Ok(draws)
}

fn validate_counter_presentation_fonts(
    assets: &CombatIndicatorAssetManifest,
    presentation: &crate::Xmap13CounterPresentation,
) -> Result<()> {
    for font_name in
        presentation
            .ordered_primitives
            .iter()
            .filter_map(|primitive| match primitive {
                crate::Xmap13CounterPrimitive::Text { font_name, .. } => Some(font_name.as_str()),
                _ => None,
            })
    {
        if !assets.fonts.iter().any(|binding| binding.name == font_name) {
            return Err(format!(
                "XMAP14 naval-grid counter font {font_name} is outside the selected-language combat closure"
            )
            .into());
        }
    }
    Ok(())
}

fn naval_counter_allegiance(
    frame: &WorldFrame,
    viewer_country: Option<&str>,
    owner_country: &str,
) -> Result<crate::Xmap13CounterAllegiance> {
    let Some(observer) = viewer_country else {
        return Ok(crate::Xmap13CounterAllegiance::WorldObserver);
    };
    if observer == owner_country {
        return Ok(crate::Xmap13CounterAllegiance::Ours);
    }
    let key = if observer < owner_country {
        (observer.to_string(), owner_country.to_string())
    } else {
        (owner_country.to_string(), observer.to_string())
    };
    let pair = frame
        .diplomacy()
        .pair(&key.0, &key.1)
        .ok_or("XMAP14 naval-grid allegiance has no exact diplomatic pair")?;
    match pair.at_war {
        Leaf::Measured(true) => Ok(crate::Xmap13CounterAllegiance::Enemy),
        Leaf::Measured(false) => Ok(crate::Xmap13CounterAllegiance::Other),
        _ => Err("XMAP14 naval-grid allegiance at-war leaf is unavailable".into()),
    }
}

fn counter_backing_modulation(presentation: &crate::Xmap13CounterPresentation) -> Result<[u32; 4]> {
    let mut matches =
        presentation
            .ordered_primitives
            .iter()
            .filter_map(|primitive| match primitive {
                crate::Xmap13CounterPrimitive::Sprite {
                    stable_layer_id,
                    modulation_rgba_f32_bits,
                    ..
                } if stable_layer_id == "backing" => Some(*modulation_rgba_f32_bits),
                _ => None,
            });
    let tint = matches
        .next()
        .ok_or("XMAP14 CTinyUnitCounter representative has no exact backing colour")?;
    if matches.next().is_some() {
        return Err("XMAP14 CTinyUnitCounter representative has ambiguous backing colour".into());
    }
    Ok(tint)
}

fn tiny_counter_draws(
    assets: &CombatIndicatorAssetManifest,
    localisation_language: &str,
    origin: [f32; 2],
    member_count: u32,
    tint: [u32; 4],
) -> Result<Vec<PreparedXmap14Draw>> {
    if member_count == 0 {
        return Err("XMAP14 CTinyUnitCounter has an empty task-force census".into());
    }
    let layout = &assets.gui_layouts.tiny_unit_counter;
    let background = required_control(layout, "background")?;
    let icon = required_control(layout, "icon")?;
    let count = required_control(layout, "count")?;
    let font_name = count
        .font
        .clone()
        .ok_or("XMAP14 CTinyUnitCounter count has no FILE font")?;
    if count.format.as_deref() != Some("center") {
        return Err("XMAP14 CTinyUnitCounter count is not authored center aligned".into());
    }
    let count_min = control_anchor(count, origin)?;
    let count_width = count
        .max_width
        .ok_or("XMAP14 CTinyUnitCounter count has no FILE maxWidth")?;
    let count_height = count
        .max_height
        .ok_or("XMAP14 CTinyUnitCounter count has no FILE maxHeight")?;
    Ok(vec![
        PreparedXmap14Draw::Sprite(sprite_draw(
            naval_sprite_for_control(assets, background)?,
            0,
            background,
            origin,
            tint,
            None,
        )?),
        PreparedXmap14Draw::Sprite(sprite_draw(
            naval_sprite_for_control(assets, icon)?,
            0,
            icon,
            origin,
            [1.0f32.to_bits(); 4],
            None,
        )?),
        PreparedXmap14Draw::Text(PreparedXmap14TextDraw {
            font_name,
            font_language: canonical_localisation_language(localisation_language)?,
            text: member_count.to_string(),
            box_min_f32_bits: count_min.map(f32::to_bits),
            box_size_f32_bits: [
                (count_width as f32).to_bits(),
                (count_height as f32).to_bits(),
            ],
        }),
    ])
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "closure", rename_all = "snake_case", deny_unknown_fields)]
pub enum Xmap14NativeDrawClosure {
    Present {
        executor_target_sha256: String,
        actual_draw_count: u32,
        ordered_draw_receipt_sha256s: Vec<String>,
    },
    AuthenticatedEmpty {
        enumerator_witness_sha256: String,
    },
    AuthenticatedCulled {
        enumerator_witness_sha256: String,
        indicator_count: u32,
        ordered_identity_sha256: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap14ExecutionReceipt {
    pub schema: String,
    pub hour_index: i64,
    pub executor_target_sha256: String,
    pub assigned_topology_sha256: String,
    pub assigned_stage_id: String,
    pub assigned_operation_id: String,
    pub assigned_ordinal: u32,
    pub assigned_assignment_sha256: String,
    pub prepared_frame_sha256: String,
    pub hit_targets_sha256: String,
    pub draw: Xmap14NativeDrawClosure,
    pub deterministic_sha256: String,
}

impl Xmap14ExecutionReceipt {
    pub fn seal(mut self) -> Result<Self> {
        self.deterministic_sha256.clear();
        self.deterministic_sha256 = Sha256::hex(&serde_json::to_vec(&self)?);
        self.validate()?;
        Ok(self)
    }

    pub fn validate_for(&self, prepared: &PreparedXmap14Frame, target: &str) -> Result<()> {
        self.validate()?;
        prepared.validate()?;
        let assignment = &prepared.combat_indicator_assignment;
        if self.hour_index != prepared.hour_index
            || self.executor_target_sha256 != target
            || self.prepared_frame_sha256 != prepared.deterministic_sha256
            || self.hit_targets_sha256 != Sha256::hex(&serde_json::to_vec(&prepared.hit_targets)?)
            || self.assigned_topology_sha256 != assignment.topology_sha256
            || self.assigned_stage_id != assignment.stage_name
            || self.assigned_operation_id != assignment.operation_name
            || self.assigned_ordinal != assignment.assigned_ordinal
            || self.assigned_assignment_sha256 != assignment.assignment_sha256
        {
            return Err("XMAP14 execution receipt differs from prepared target frame".into());
        }
        match (&self.draw, prepared.indicators.is_empty()) {
            (
                Xmap14NativeDrawClosure::AuthenticatedEmpty {
                    enumerator_witness_sha256,
                },
                true,
            ) if enumerator_witness_sha256 == &prepared.enumerator_witness_sha256 => Ok(()),
            (
                Xmap14NativeDrawClosure::AuthenticatedCulled {
                    enumerator_witness_sha256,
                    indicator_count,
                    ordered_identity_sha256,
                },
                false,
            ) if prepared
                .indicators
                .iter()
                .all(|indicator| indicator.ordered_draws.is_empty())
                && enumerator_witness_sha256 == &prepared.enumerator_witness_sha256
                && *indicator_count == prepared.indicators.len() as u32
                && ordered_identity_sha256 == &prepared_identity_sha256(prepared) =>
            {
                Ok(())
            }
            (
                Xmap14NativeDrawClosure::Present {
                    actual_draw_count, ..
                },
                false,
            ) if *actual_draw_count
                >= prepared
                    .indicators
                    .iter()
                    .map(|indicator| indicator.ordered_draws.len() as u32)
                    .sum() =>
            {
                Ok(())
            }
            _ => Err("XMAP14 execution closure disagrees with prepared draw census".into()),
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP14_NATIVE_RECEIPT_SCHEMA {
            return Err("XMAP14 execution receipt schema mismatch".into());
        }
        for (value, role) in [
            (&self.executor_target_sha256, "XMAP14 executor target"),
            (&self.assigned_topology_sha256, "XMAP14 assigned topology"),
            (
                &self.assigned_assignment_sha256,
                "XMAP14 assigned assignment",
            ),
            (&self.prepared_frame_sha256, "XMAP14 prepared frame"),
            (&self.hit_targets_sha256, "XMAP14 hit targets"),
            (&self.deterministic_sha256, "XMAP14 execution receipt"),
        ] {
            validate_sha256(value, role)?;
        }
        if self.assigned_stage_id != XMAP14_COMBAT_STAGE_NAME
            || self.assigned_operation_id != XMAP14_COMBAT_OPERATION_NAME
        {
            return Err("XMAP14 execution receipt assignment identity mismatch".into());
        }
        match &self.draw {
            Xmap14NativeDrawClosure::Present {
                executor_target_sha256,
                actual_draw_count,
                ordered_draw_receipt_sha256s,
            } => {
                validate_sha256(executor_target_sha256, "XMAP14 draw target")?;
                if executor_target_sha256 != &self.executor_target_sha256
                    || *actual_draw_count == 0
                    || ordered_draw_receipt_sha256s.len() != *actual_draw_count as usize
                {
                    return Err("XMAP14 present draw receipt census mismatch".into());
                }
                for receipt in ordered_draw_receipt_sha256s {
                    validate_sha256(receipt, "XMAP14 ordered draw receipt")?;
                }
            }
            Xmap14NativeDrawClosure::AuthenticatedEmpty {
                enumerator_witness_sha256,
            } => validate_sha256(enumerator_witness_sha256, "XMAP14 empty witness")?,
            Xmap14NativeDrawClosure::AuthenticatedCulled {
                enumerator_witness_sha256,
                indicator_count,
                ordered_identity_sha256,
            } => {
                validate_sha256(enumerator_witness_sha256, "XMAP14 culled witness")?;
                validate_sha256(ordered_identity_sha256, "XMAP14 culled identity order")?;
                if *indicator_count == 0 {
                    return Err("XMAP14 culled closure has an empty semantic census".into());
                }
            }
        }
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        if self.deterministic_sha256 != Sha256::hex(&serde_json::to_vec(&unsigned)?) {
            return Err("XMAP14 execution receipt digest mismatch".into());
        }
        Ok(())
    }
}

fn required_size(layout: &crate::GuiContainerLayout) -> Result<crate::GuiSize> {
    let size = layout
        .size
        .ok_or_else(|| format!("XMAP14 container {} has no FILE size", layout.name))?;
    if size.width <= 0 || size.height <= 0 {
        return Err(format!("XMAP14 container {} has invalid FILE size", layout.name).into());
    }
    Ok(size)
}

fn required_control<'a>(
    layout: &'a crate::GuiContainerLayout,
    name: &str,
) -> Result<&'a crate::GuiControlLayout> {
    layout
        .control(name)
        .ok_or_else(|| format!("XMAP14 container {} omitted control {name}", layout.name).into())
}

fn required_land_sprite<'a>(
    assets: &'a CombatIndicatorAssetManifest,
    name: &str,
) -> Result<&'a crate::CounterSpriteAsset> {
    assets
        .land_combat_sprite(name)
        .ok_or_else(|| format!("XMAP14 effective VFS omitted land sprite {name}").into())
}

fn required_naval_sprite<'a>(
    assets: &'a CombatIndicatorAssetManifest,
    name: &str,
) -> Result<&'a crate::CounterSpriteAsset> {
    assets
        .naval_combat_sprite(name)
        .ok_or_else(|| format!("XMAP14 effective VFS omitted naval sprite {name}").into())
}

fn one_based_frame_index(frame: u8) -> Result<u32> {
    u32::from(frame)
        .checked_sub(1)
        .ok_or_else(|| "XMAP14 GUI frame number must be one-based".into())
}

fn aligned_container_origin(screen: [f32; 2], size: crate::GuiSize) -> Result<[f32; 2]> {
    if !screen.iter().all(|value| value.is_finite()) {
        return Err("XMAP14 projected screen coordinate is non-finite".into());
    }
    Ok([
        signed_half_round(screen[0] - size.width as f32 * 0.5),
        signed_half_round(screen[1] - size.height as f32 * 0.5),
    ])
}

fn signed_half_round(value: f32) -> f32 {
    if value < 0.0 {
        (value - 0.5).ceil()
    } else {
        (value + 0.5).floor()
    }
}

fn sprite_draw(
    asset: &crate::CounterSpriteAsset,
    frame_index: u32,
    control: &crate::GuiControlLayout,
    container_origin: [f32; 2],
    modulation_rgba_f32_bits: [u32; 4],
    rotation_radians: Option<f32>,
) -> Result<PreparedXmap14SpriteDraw> {
    let frame = asset
        .frame(frame_index)
        .ok_or_else(|| format!("XMAP14 sprite {} omitted frame {frame_index}", asset.name))?;
    let size = crate::GuiSize {
        width: i32::try_from(frame.rect.width)
            .map_err(|_| "XMAP14 sprite width exceeds GUI range")?,
        height: i32::try_from(frame.rect.height)
            .map_err(|_| "XMAP14 sprite height exceeds GUI range")?,
    };
    let min = control_rect_min(control, container_origin, size)?;
    let mut positions = axis_aligned_strip(min, size)?;
    if let Some(angle) = rotation_radians {
        if !angle.is_finite() {
            return Err("XMAP14 sprite rotation is non-finite".into());
        }
        let center = [
            min[0] + size.width as f32 * 0.5,
            min[1] + size.height as f32 * 0.5,
        ];
        let (sin, cos) = angle.sin_cos();
        for point in &mut positions {
            let x = point[0] - center[0];
            let y = point[1] - center[1];
            point[0] = center[0] + x * cos - y * sin;
            point[1] = center[1] + x * sin + y * cos;
        }
    }
    let uv = [
        [frame.uv.u0, frame.uv.v0],
        [frame.uv.u0, frame.uv.v1],
        [frame.uv.u1, frame.uv.v0],
        [frame.uv.u1, frame.uv.v1],
    ];
    Ok(PreparedXmap14SpriteDraw {
        texture_file: asset.texture_file.clone(),
        frame_index,
        positions_f32_bits: positions.map(|value| value.map(f32::to_bits)),
        uv_f32_bits: uv.map(|value| value.map(f32::to_bits)),
        modulation_rgba_f32_bits,
    })
}

fn axis_aligned_strip(min: [f32; 2], size: crate::GuiSize) -> Result<[[f32; 2]; 4]> {
    if size.width <= 0 || size.height <= 0 || !min.iter().all(|value| value.is_finite()) {
        return Err("XMAP14 draw rectangle is invalid".into());
    }
    let max = [min[0] + size.width as f32, min[1] + size.height as f32];
    Ok([
        [min[0], min[1]],
        [min[0], max[1]],
        [max[0], min[1]],
        [max[0], max[1]],
    ])
}

fn full_uv_strip() -> [[f32; 2]; 4] {
    [[0.0, 0.0], [0.0, 1.0], [1.0, 0.0], [1.0, 1.0]]
}

fn validate_land(row: &Xmap14LandCarrier) -> Result<()> {
    if !matches!(row.identity, Xmap14IndicatorIdentity::Land { .. })
        || row.location_province_id == 0
        || row.progress_fixed5 < 0
        || row.progress_fixed5 > 100_000
        || row.attacker_comparison_fixed < 0
        || row.defender_comparison_fixed < 0
        || row.frame_selector > 1
        || row.attackers.is_empty()
        || row.defenders.is_empty()
        || !row.active_phase
    {
        return Err("XMAP14 land carrier is incomplete or invalid".into());
    }
    row.identity.validate()?;
    point(row.combat_base_point_f32_bits)?;
    point(row.first_live_attacker_stack_point_f32_bits)?;
    validate_unit_sides(&row.attackers, &row.defenders, false)?;
    validate_sha256(&row.side_flags_sha256, "XMAP14 land side flags")?;
    let expected = Sha256::hex(
        &[
            row.attacker_side_flags.to_le_bytes(),
            row.defender_side_flags.to_le_bytes(),
        ]
        .concat(),
    );
    if row.side_flags_sha256 != expected {
        return Err("XMAP14 land side-flags digest mismatch".into());
    }
    Ok(())
}

fn validate_naval(row: &Xmap14NavalCarrier) -> Result<()> {
    if !matches!(row.identity, Xmap14IndicatorIdentity::Naval { .. })
        || row.location_province_id == 0
        || row.progress_fixed5 < 0
        || row.progress_fixed5 > 100_000
        || !row.active_phase
    {
        return Err("XMAP14 naval carrier is incomplete or invalid".into());
    }
    row.identity.validate()?;
    point(row.anchor_point_f32_bits)?;
    validate_unit_sides(&row.attackers, &row.defenders, true)?;
    validate_sha256(&row.category_counts_sha256, "XMAP14 naval category counts")?;
    let bytes = row
        .attacker_category_counts
        .ordered_counts
        .iter()
        .chain(row.defender_category_counts.ordered_counts.iter())
        .flat_map(|value| value.to_le_bytes())
        .collect::<Vec<_>>();
    if row.category_counts_sha256 != Sha256::hex(&bytes) {
        return Err("XMAP14 naval category-count digest mismatch".into());
    }
    Ok(())
}

fn validate_naval_result(row: &Xmap14NavalResultCarrier) -> Result<()> {
    if !matches!(row.identity, Xmap14IndicatorIdentity::NavalResult { .. })
        || row.location_province_id == 0
        || row.members.is_empty()
        || row.retire_sequence.is_some_and(|value| value == 0)
    {
        return Err("XMAP14 naval-result carrier is incomplete or invalid".into());
    }
    row.identity.validate()?;
    point(row.anchor_point_f32_bits)?;
    validate_sha256(&row.lifecycle_sha256, "XMAP14 naval-result lifecycle")?;
    let mut member_ids = BTreeSet::new();
    for member in &row.members {
        if member.member_id <= 0
            || !member_ids.insert(member.member_id)
            || member.first_side_countries.is_empty()
            || member.second_side_countries.is_empty()
        {
            return Err("XMAP14 naval-result member is incomplete or duplicated".into());
        }
        validate_country_set(&member.first_side_countries)?;
        validate_country_set(&member.second_side_countries)?;
    }
    let identity = serde_json::to_vec(&(
        &row.identity,
        row.location_province_id,
        row.alternate_anchor_branch,
        row.anchor_point_f32_bits,
        &row.members,
        row.retire_sequence,
    ))?;
    if row.lifecycle_sha256 != Sha256::hex(&identity) {
        return Err("XMAP14 naval-result lifecycle digest mismatch".into());
    }
    Ok(())
}

fn validate_country_set(values: &[String]) -> Result<()> {
    let mut seen = BTreeSet::new();
    if values
        .iter()
        .any(|value| value.trim().is_empty() || !seen.insert(value.as_str()))
    {
        return Err("XMAP14 country set is blank or duplicated".into());
    }
    Ok(())
}

fn validate_unit_sides(
    first: &[Xmap13UnitIdentity],
    second: &[Xmap13UnitIdentity],
    naval: bool,
) -> Result<()> {
    let mut seen = BTreeSet::new();
    for identity in first.iter().chain(second.iter()) {
        let expected_kind = if naval {
            matches!(identity, Xmap13UnitIdentity::TaskForce { .. })
        } else {
            matches!(identity, Xmap13UnitIdentity::Division { .. })
        };
        if !expected_kind || !seen.insert(identity) {
            return Err("XMAP14 participant identity kind/order is invalid".into());
        }
    }
    if !naval && (first.is_empty() || second.is_empty()) {
        return Err("XMAP14 combat side is measured-empty".into());
    }
    Ok(())
}

fn validate_typed_census(
    frame: &WorldFrame,
    envelope: &Xmap14CombatEnvelope,
    provider: &crate::Xmap13IdentityProvider,
) -> Result<()> {
    if frame.combats_status() != ViewStatus::Present
        || frame.naval_air_combat().naval_combats_status != ViewStatus::Present
    {
        return Err("XMAP14 typed combat collections are not complete/Present".into());
    }
    let land_rows = frame.combats();
    validate_typed_land_census(land_rows, &envelope.land)?;

    let typed_naval = &frame.naval_air_combat().naval_combats;
    if typed_naval.len() != envelope.naval.len() {
        return Err("XMAP14 naval carrier does not cover the typed active census".into());
    }
    for (typed, carrier) in typed_naval.iter().zip(&envelope.naval) {
        let (Leaf::Measured(combat_id), Leaf::Measured(generation)) =
            (&typed.combat_id, &typed.generation)
        else {
            return Err("XMAP14 typed naval identity is unavailable".into());
        };
        if carrier.identity
            != (Xmap14IndicatorIdentity::Naval {
                combat_id: *combat_id,
                generation: *generation,
            })
            || typed.location_province.measured() != Some(&carrier.location_province_id)
        {
            return Err("XMAP14 naval carrier identity/location join mismatch".into());
        }
        validate_bare_taskforce_join(&typed.attacker_task_force_ids, &carrier.attackers)?;
        validate_bare_taskforce_join(&typed.defender_task_force_ids, &carrier.defenders)?;
    }

    for identity in envelope
        .land
        .iter()
        .flat_map(|row| row.attackers.iter().chain(row.defenders.iter()))
        .chain(
            envelope
                .naval
                .iter()
                .flat_map(|row| row.attackers.iter().chain(row.defenders.iter())),
        )
    {
        if provider.lookup(identity).is_none() {
            return Err("XMAP14 participant is absent from the XMAP13 census".into());
        }
    }
    Ok(())
}

fn validate_typed_land_census(
    land_rows: &[CombatRow],
    carriers: &[Xmap14LandCarrier],
) -> Result<()> {
    if land_rows
        .iter()
        .any(|row| !matches!(&row.kind, Leaf::Measured(kind) if kind == "land"))
    {
        return Err(
            "XMAP14 typed active combat census contains an unavailable or non-land kind".into(),
        );
    }
    if land_rows.len() != carriers.len() {
        return Err("XMAP14 land carrier does not cover the typed active census".into());
    }
    for (typed, carrier) in land_rows.iter().zip(carriers) {
        let (combat_id, generation) = match (&typed.combat_id, &typed.generation) {
            (Leaf::Measured(id), Leaf::Measured(generation)) => (*id, *generation),
            _ => return Err("XMAP14 typed land identity is unavailable".into()),
        };
        if carrier.identity
            != (Xmap14IndicatorIdentity::Land {
                combat_id,
                generation,
            })
            || typed.location_province.measured() != Some(&carrier.location_province_id)
        {
            return Err("XMAP14 land carrier identity/location join mismatch".into());
        }
        validate_bare_division_join(&typed.attackers, &carrier.attackers)?;
        validate_bare_division_join(&typed.defenders, &carrier.defenders)?;
    }
    Ok(())
}

fn validate_bare_division_join(
    typed: &Leaf<Vec<i64>>,
    carrier: &[Xmap13UnitIdentity],
) -> Result<()> {
    let Leaf::Measured(typed) = typed else {
        return Err("XMAP14 typed land participant vector is unavailable".into());
    };
    let carried = carrier
        .iter()
        .map(|identity| match identity {
            Xmap13UnitIdentity::Division { stable_id, .. } => Ok(i64::from(*stable_id)),
            _ => Err("XMAP14 land participant is not a division"),
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if *typed != carried {
        return Err("XMAP14 land participant order disagrees with typed d21".into());
    }
    Ok(())
}

fn validate_bare_taskforce_join(
    typed: &Leaf<Vec<i64>>,
    carrier: &[Xmap13UnitIdentity],
) -> Result<()> {
    let Leaf::Measured(typed) = typed else {
        return Err("XMAP14 typed naval task-force vector is unavailable".into());
    };
    let carried = carrier
        .iter()
        .map(|identity| match identity {
            Xmap13UnitIdentity::TaskForce { stable_id, .. } => Ok(i64::from(*stable_id)),
            _ => Err("XMAP14 naval participant is not a task force"),
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if *typed != carried {
        return Err("XMAP14 naval task-force order disagrees with typed D50".into());
    }
    Ok(())
}

fn typed_naval_combat<'a>(
    frame: &'a WorldFrame,
    identity: &Xmap14IndicatorIdentity,
) -> Result<&'a replay_core::NavalCombatRow> {
    let Xmap14IndicatorIdentity::Naval {
        combat_id,
        generation,
    } = identity
    else {
        return Err("XMAP14 requested a typed naval row for a non-naval identity".into());
    };
    frame
        .naval_air_combat()
        .naval_combats
        .iter()
        .find(|row| {
            row.combat_id.measured() == Some(combat_id)
                && row.generation.measured() == Some(generation)
        })
        .ok_or_else(|| "XMAP14 naval carrier is absent from the typed D50 census".into())
}

fn viewer_side_for_naval(
    viewer_country: Option<&str>,
    typed: &replay_core::NavalCombatRow,
) -> Result<Xmap14ViewerSide> {
    let Some(viewer_country) = viewer_country else {
        return Ok(Xmap14ViewerSide::First);
    };
    let attacker = typed
        .attacker
        .measured()
        .ok_or("XMAP14 typed naval attacker country is unavailable")?;
    let defender = typed
        .defender
        .measured()
        .ok_or("XMAP14 typed naval defender country is unavailable")?;
    viewer_side_for_country_pair(viewer_country, attacker, defender)
}

fn viewer_side_for_country_pair(
    viewer_country: &str,
    attacker: &str,
    defender: &str,
) -> Result<Xmap14ViewerSide> {
    match (attacker == viewer_country, defender == viewer_country) {
        (true, false) => Ok(Xmap14ViewerSide::First),
        (false, true) => Ok(Xmap14ViewerSide::Second),
        (false, false) => Ok(Xmap14ViewerSide::First),
        (true, true) => Err("XMAP14 typed naval viewer country occurs on both sides".into()),
    }
}

fn viewer_side_for_units(
    viewer_country: Option<&str>,
    first: &[Xmap13UnitIdentity],
    second: &[Xmap13UnitIdentity],
    provider: &crate::Xmap13IdentityProvider,
) -> Result<Xmap14ViewerSide> {
    let Some(viewer_country) = viewer_country else {
        return Ok(Xmap14ViewerSide::First);
    };
    let first_match = side_has_country(first, viewer_country, provider)?;
    let second_match = side_has_country(second, viewer_country, provider)?;
    match (first_match, second_match) {
        (true, false) => Ok(Xmap14ViewerSide::First),
        (false, true) => Ok(Xmap14ViewerSide::Second),
        (false, false) => Ok(Xmap14ViewerSide::First),
        (true, true) => Err("XMAP14 viewer country occurs on both combat sides".into()),
    }
}

fn side_has_country(
    identities: &[Xmap13UnitIdentity],
    country: &str,
    provider: &crate::Xmap13IdentityProvider,
) -> Result<bool> {
    let mut matched = false;
    for identity in identities {
        let row = provider
            .lookup(identity)
            .ok_or("XMAP14 participant is absent from XMAP13")?;
        matched |= row.owner_country == country;
    }
    Ok(matched)
}

fn viewer_progress(progress: i64, side: Xmap14ViewerSide) -> i64 {
    match side {
        Xmap14ViewerSide::First => progress,
        Xmap14ViewerSide::Second => 100_000 - progress,
    }
}

fn land_tint(
    attacker: i64,
    defender: i64,
    viewer_side: Xmap14ViewerSide,
) -> Result<Xmap14LandTint> {
    let (ours, theirs) = match viewer_side {
        Xmap14ViewerSide::First => (attacker, defender),
        Xmap14ViewerSide::Second => (defender, attacker),
    };
    let ours = i128::from(ours);
    let theirs = i128::from(theirs);
    if theirs * 110_000 / 100_000 < ours {
        Ok(Xmap14LandTint::Winning)
    } else if ours * 110_000 / 100_000 < theirs {
        Ok(Xmap14LandTint::Losing)
    } else {
        Ok(Xmap14LandTint::Balanced)
    }
}

fn naval_convoy_frame(first: &[u32; 7], second: &[u32; 7]) -> bool {
    let first_surface = first[6] + first[4] + first[2] + first[1] + first[0];
    let second_surface = second[6] + second[4] + second[2] + second[1] + second[0];
    if first[5] >= 1 && (first_surface == 0 || (second_surface == 0 && second[3] >= 1)) {
        return true;
    }
    if second[5] >= 1 && (second_surface == 0 || (first_surface == 0 && first[3] >= 1)) {
        return true;
    }
    false
}

fn naval_result_frame(row: &Xmap14NavalResultCarrier, viewer_country: Option<&str>) -> Result<u8> {
    let all_ready = row.members.iter().all(|member| member.frame_state_ready);
    if all_ready {
        return Ok(3);
    }
    let last_predicate = row
        .members
        .last()
        .ok_or("XMAP14 naval result has no member")?
        .result_predicate;
    let viewer_exclusive_first = if let Some(country) = viewer_country {
        row.members.iter().any(|member| {
            member
                .first_side_countries
                .iter()
                .any(|value| value == country)
                && !member
                    .second_side_countries
                    .iter()
                    .any(|value| value == country)
        })
    } else {
        false
    };
    Ok(match (last_predicate, viewer_exclusive_first) {
        (false, false) => 1,
        (true, true) => 2,
        _ => 3,
    })
}

fn midpoint(first: [u32; 3], second: [u32; 3]) -> [f32; 3] {
    let first = first.map(f32::from_bits);
    let second = second.map(f32::from_bits);
    [
        (first[0] + second[0]) * 0.5,
        (first[1] + second[1]) * 0.5,
        (first[2] + second[2]) * 0.5,
    ]
}

fn direction_radians(first: [u32; 3], second: [u32; 3]) -> Result<f32> {
    let first = point(first)?;
    let second = point(second)?;
    Ok((second[0] - first[0]).atan2(-(second[2] - first[2])))
}

fn point(bits: [u32; 3]) -> Result<[f32; 3]> {
    let point = bits.map(f32::from_bits);
    if !point.iter().all(|value| value.is_finite()) {
        return Err("XMAP14 world point is non-finite".into());
    }
    Ok(point)
}

fn project_map_icon(
    camera: &crate::ProductionMapCamera,
    point: [f32; 3],
    viewer: &crate::ViewerOwnedMapState,
    container_size: crate::GuiSize,
) -> Result<Option<[u32; 3]>> {
    let Some(screen) = camera.world_to_screen_depth(
        point,
        viewer.viewport_width as f32,
        viewer.viewport_height as f32,
    ) else {
        return Ok(None);
    };
    if container_size.width <= 0 || container_size.height <= 0 {
        return Err("XMAP14 mapicon container has an invalid size".into());
    }
    let [rounded_x, rounded_y] = aligned_mapicon_origin(screen, container_size)?;
    if !aligned_rect_intersects_viewport(
        [rounded_x, rounded_y],
        container_size,
        [viewer.viewport_width, viewer.viewport_height],
    ) {
        return Ok(None);
    }
    Ok(Some([
        screen[0].to_bits(),
        screen[1].to_bits(),
        screen[2].to_bits(),
    ]))
}

fn aligned_mapicon_origin(screen: [f32; 3], container_size: crate::GuiSize) -> Result<[f32; 2]> {
    if container_size.width <= 0
        || container_size.height <= 0
        || !screen.iter().all(|value| value.is_finite())
    {
        return Err("XMAP14 mapicon alignment input is invalid".into());
    }
    let aligned_x = screen[0] - container_size.width as f32 * 0.5;
    let aligned_y = screen[1] - container_size.height as f32 * 0.5;
    let rounded_x = if aligned_x < 0.0 {
        (aligned_x - 0.5).ceil()
    } else {
        (aligned_x + 0.5).floor()
    };
    let rounded_y = if aligned_y < 0.0 {
        (aligned_y - 0.5).ceil()
    } else {
        (aligned_y + 0.5).floor()
    };
    Ok([rounded_x, rounded_y])
}

fn aligned_rect_intersects_viewport(
    origin: [f32; 2],
    container_size: crate::GuiSize,
    viewport: [u32; 2],
) -> bool {
    origin[0] + container_size.width as f32 > 0.0
        && origin[1] + container_size.height as f32 > 0.0
        && origin[0] < viewport[0] as f32
        && origin[1] < viewport[1] as f32
}

fn admitted_content_closure(
    resolver: &GameFileResolver,
    admission: &ValidatedContentAdmission,
    assets: &CombatIndicatorAssetManifest,
    graphics_defines: UsedContentKey,
    localisation_paths: &[String],
    progress_shaders: &BTreeMap<String, ProgressShaderBinding>,
    localisation_language: &str,
) -> Result<Vec<UsedContentKey>> {
    let mut paths = REQUIRED_INTERFACE_SOURCES
        .iter()
        .map(|value| (*value).to_string())
        .collect::<Vec<_>>();
    paths.push(GRAPHICS_DEFINES.into());
    paths.extend(localisation_paths.iter().cloned());
    paths.extend(
        assets
            .land_combat_sprites
            .values()
            .chain(assets.naval_combat_sprites.values())
            .map(|asset| asset.texture_file.clone()),
    );
    paths.extend(
        assets
            .progress_bars
            .values()
            .flat_map(|asset| [asset.texture_file1.clone(), asset.texture_file2.clone()]),
    );
    paths.extend(
        assets
            .land_combat_sprites
            .values()
            .chain(assets.naval_combat_sprites.values())
            .map(|asset| asset.source_file.clone()),
    );
    paths.extend(
        assets
            .progress_bars
            .values()
            .map(|asset| asset.source_file.clone()),
    );
    paths.extend(
        progress_shaders
            .values()
            .map(|binding| binding.shader_file.clone()),
    );
    paths.extend(combat_font_content_paths(
        resolver,
        admission,
        assets,
        localisation_language,
    )?);
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for path in paths {
        let path = path.replace('\\', "/");
        if seen.insert(path.clone()) {
            out.push(
                admission
                    .require_used_content(&path)
                    .map_err(|error| format!("XMAP14 effective-VFS closure lacks {path}: {error}"))?
                    .clone(),
            );
        }
    }
    if !out.iter().any(|value| value == &graphics_defines) {
        return Err("XMAP14 graphics defines disappeared from content closure".into());
    }
    Ok(out)
}

fn control_rect_min(
    control: &crate::GuiControlLayout,
    container_origin: [f32; 2],
    size: crate::GuiSize,
) -> Result<[f32; 2]> {
    let mut min = control_anchor(control, container_origin)?;
    if control.center_position == Some(true) {
        min[0] -= size.width as f32 * 0.5;
        min[1] -= size.height as f32 * 0.5;
    }
    Ok(min)
}

fn required_control_alias(control: &crate::GuiControlLayout) -> Result<&str> {
    match (
        control.sprite_type.as_deref(),
        control.quad_texture_sprite.as_deref(),
    ) {
        (Some(left), Some(right)) if left != right => Err(format!(
            "XMAP14 control {} declares conflicting sprite aliases",
            control.name
        )
        .into()),
        (Some(value), _) | (_, Some(value)) => Ok(value),
        (None, None) => Err(format!(
            "XMAP14 drawable control {} has no effective sprite alias",
            control.name
        )
        .into()),
    }
}

fn land_sprite_for_control<'a>(
    assets: &'a CombatIndicatorAssetManifest,
    control: &crate::GuiControlLayout,
) -> Result<&'a crate::CounterSpriteAsset> {
    required_land_sprite(assets, required_control_alias(control)?)
}

fn naval_sprite_for_control<'a>(
    assets: &'a CombatIndicatorAssetManifest,
    control: &crate::GuiControlLayout,
) -> Result<&'a crate::CounterSpriteAsset> {
    required_naval_sprite(assets, required_control_alias(control)?)
}

fn control_anchor(
    control: &crate::GuiControlLayout,
    container_origin: [f32; 2],
) -> Result<[f32; 2]> {
    let local = if let Some(bits) = control.effective_position_f32_bits {
        bits.map(f32::from_bits)
    } else {
        let position = control
            .position
            .ok_or_else(|| format!("XMAP14 control {} has no FILE position", control.name))?;
        [position.x as f32, position.y as f32]
    };
    let orientation_offset = match control.orientation.as_deref() {
        None | Some("upper_left") => [0.0, 0.0],
        Some(value) if value.eq_ignore_ascii_case("upper_left") => [0.0, 0.0],
        Some(value) if value.eq_ignore_ascii_case("center") => {
            let parent = control.parent_size.ok_or_else(|| {
                format!(
                    "XMAP14 centered control {} has no parent size",
                    control.name
                )
            })?;
            if parent.width <= 0 || parent.height <= 0 {
                return Err(format!(
                    "XMAP14 centered control {} has an invalid parent size",
                    control.name
                )
                .into());
            }
            [parent.width as f32 * 0.5, parent.height as f32 * 0.5]
        }
        Some(value) => {
            return Err(format!(
                "XMAP14 control {} uses unsupported orientation {value}",
                control.name
            )
            .into())
        }
    };
    let anchor = [
        container_origin[0] + local[0] + orientation_offset[0],
        container_origin[1] + local[1] + orientation_offset[1],
    ];
    if anchor.iter().all(|value| value.is_finite()) {
        Ok(anchor)
    } else {
        Err("XMAP14 control anchor is non-finite".into())
    }
}

fn canonical_localisation_language(value: &str) -> Result<String> {
    let mut language = value.trim().to_ascii_lowercase();
    if !language.starts_with("l_") {
        language = format!("l_{language}");
    }
    if language.len() <= 2
        || !language[2..]
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
    {
        return Err(format!("XMAP14 localisation language is not canonical: {value:?}").into());
    }
    Ok(language)
}

fn select_combat_font_binding<'a>(
    assets: &'a CombatIndicatorAssetManifest,
    font_name: &str,
    localisation_language: &str,
) -> Result<&'a crate::unit_counter_assets::BitmapFontBinding> {
    let language = canonical_localisation_language(localisation_language)?;
    let mut language_matches = assets.fonts.iter().filter(|font| {
        font.name == font_name
            && font
                .languages
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(&language))
    });
    if let Some(binding) = language_matches.next() {
        if language_matches.next().is_some() {
            return Err(
                format!("XMAP14 has ambiguous playback-language {font_name} overrides").into(),
            );
        }
        return Ok(binding);
    }
    let mut defaults = assets
        .fonts
        .iter()
        .filter(|font| font.name == font_name && font.languages.is_empty());
    let binding = defaults
        .next()
        .ok_or_else(|| format!("XMAP14 has no default {font_name} bitmapfont binding"))?;
    if defaults.next().is_some() {
        return Err(format!("XMAP14 has ambiguous default {font_name} bindings").into());
    }
    Ok(binding)
}

fn progress_shader_bindings(
    resolver: &GameFileResolver,
    admission: &ValidatedContentAdmission,
    assets: &CombatIndicatorAssetManifest,
) -> Result<BTreeMap<String, ProgressShaderBinding>> {
    let mut out = BTreeMap::new();
    for (name, bar) in &assets.progress_bars {
        if bar.horizontal != Some(true) {
            return Err(format!("XMAP14 progress bar {name} is not authored horizontal").into());
        }
        let effect = bar
            .effect_file
            .as_deref()
            .ok_or_else(|| format!("XMAP14 progress bar {name} has no effectFile"))?;
        let mut shader_file = normalize_game_relative_path(effect)
            .map_err(|error| format!("XMAP14 progress effect path: {error}"))?;
        if shader_file.to_ascii_lowercase().ends_with(".lua") {
            shader_file.truncate(shader_file.len() - 4);
            shader_file.push_str(".shader");
        }
        if !shader_file.to_ascii_lowercase().ends_with(".shader") {
            return Err(
                format!("XMAP14 progress effect is not a Clausewitz shader: {effect}").into(),
            );
        }
        let key = admission
            .require_used_content(&shader_file)
            .map_err(|error| format!("XMAP14 progress shader is not admitted: {error}"))?
            .clone();
        let bytes = resolver.read_admitted_used_content(&key)?;
        validate_supported_progress_shader(&bytes, &shader_file)?;
        out.insert(
            name.clone(),
            ProgressShaderBinding {
                shader_file,
                shader_sha256: Sha256::hex(&bytes),
            },
        );
    }
    Ok(out)
}

fn validate_supported_progress_shader(bytes: &[u8], source: &str) -> Result<()> {
    let compact = normalized_progress_shader_source(&latin1(bytes))?;
    let supported = normalized_progress_shader_source(SUPPORTED_PROGRESS_SHADER_SOURCE)?;
    if compact != supported {
        return Err(format!(
            "XMAP14 progress shader {source} is outside the exact supported Texture effect"
        )
        .into());
    }
    Ok(())
}

fn normalized_progress_shader_source(source: &str) -> Result<Vec<String>> {
    let source = strip_shader_comments(source)?;
    if !source.is_ascii() {
        return Err("XMAP14 progress shader contains an unsupported non-ASCII token".into());
    }
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index].is_ascii_whitespace() {
            index += 1;
            continue;
        }
        let start = index;
        if bytes[index] == b'"' {
            index += 1;
            let mut escaped = false;
            while index < bytes.len() {
                let byte = bytes[index];
                index += 1;
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'"' {
                    break;
                }
            }
        } else if bytes[index].is_ascii_alphabetic() || bytes[index] == b'_' {
            index += 1;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
        } else if bytes[index].is_ascii_digit() {
            index += 1;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
            if index + 1 < bytes.len() && bytes[index] == b'.' && bytes[index + 1].is_ascii_digit()
            {
                index += 1;
                while index < bytes.len() && bytes[index].is_ascii_digit() {
                    index += 1;
                }
            }
            if index < bytes.len() && matches!(bytes[index], b'e' | b'E') {
                let exponent = index;
                index += 1;
                if index < bytes.len() && matches!(bytes[index], b'+' | b'-') {
                    index += 1;
                }
                let digits = index;
                while index < bytes.len() && bytes[index].is_ascii_digit() {
                    index += 1;
                }
                if index == digits {
                    index = exponent;
                }
            }
        } else if index + 1 < bytes.len()
            && matches!(
                (bytes[index], bytes[index + 1]),
                (b'[', b'[')
                    | (b']', b']')
                    | (b'<', b'=')
                    | (b'>', b'=')
                    | (b'=', b'=')
                    | (b'!', b'=')
                    | (b'&', b'&')
                    | (b'|', b'|')
            )
        {
            index += 2;
        } else {
            index += 1;
        }
        tokens.push(source[start..index].to_ascii_lowercase());
    }
    Ok(tokens)
}

fn strip_shader_comments(source: &str) -> Result<String> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum State {
        Code,
        Quoted,
        Line,
        Block,
    }
    let bytes = source.as_bytes();
    let mut state = State::Code;
    let mut escaped = false;
    let mut out = String::with_capacity(source.len());
    let mut index = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        let next = bytes.get(index + 1).copied();
        match state {
            State::Code if byte == b'"' => {
                state = State::Quoted;
                out.push('"');
            }
            State::Code if byte == b'/' && next == Some(b'/') => {
                state = State::Line;
                index += 1;
            }
            State::Code if byte == b'/' && next == Some(b'*') => {
                state = State::Block;
                out.push(' ');
                index += 1;
            }
            State::Code => out.push(char::from(byte)),
            State::Quoted => {
                out.push(char::from(byte));
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'"' {
                    state = State::Code;
                }
            }
            State::Line if byte == b'\n' => {
                state = State::Code;
                out.push('\n');
            }
            State::Line => {}
            State::Block if byte == b'*' && next == Some(b'/') => {
                state = State::Code;
                index += 1;
            }
            State::Block if byte == b'\n' => out.push('\n'),
            State::Block => {}
        }
        index += 1;
    }
    if state == State::Block || state == State::Quoted {
        return Err("XMAP14 progress shader has an unterminated comment or string".into());
    }
    Ok(out)
}

fn combat_progress_localisation(
    resolver: &GameFileResolver,
    admission: &ValidatedContentAdmission,
    localisation_language: &str,
) -> Result<(String, Vec<String>)> {
    let canonical = canonical_localisation_language(localisation_language)?;
    let language = canonical.trim_start_matches("l_").to_string();
    let directory = format!("localisation/{language}");
    let files = resolver.resolve_dir_files(&directory, Some("yml"))?;
    if files.is_empty() {
        return Err(
            format!("XMAP14 effective localisation directory is empty: {directory}").into(),
        );
    }
    let mut table = std::collections::BTreeMap::new();
    let mut paths = Vec::with_capacity(files.len());
    for (path, _) in files {
        let key = admission
            .require_used_content(&path)
            .map_err(|error| format!("XMAP14 requires admitted localisation {path}: {error}"))?
            .clone();
        let bytes = resolver.read_admitted_used_content(&key)?;
        let source = std::str::from_utf8(&bytes)
            .map_err(|error| format!("XMAP14 localisation {path} is not UTF-8: {error}"))?;
        crate::merge_localisation_source_exact(
            &mut table,
            source,
            &path,
            &format!("{canonical}:"),
        )?;
        paths.push(path);
    }
    let template = table
        .get("COMBAT_ICON_PROGRESS")
        .ok_or("XMAP14 effective localisation has no COMBAT_ICON_PROGRESS")?
        .clone();
    if template.matches("$VAL|0$").count() != 1 || template.replace("$VAL|0$", "").contains('$') {
        return Err(
            "XMAP14 COMBAT_ICON_PROGRESS uses an unsupported localisation substitution law".into(),
        );
    }
    Ok((template, paths))
}

fn format_combat_progress(template: &str, progress_fixed5: i64) -> Result<String> {
    if !(0..=100_000).contains(&progress_fixed5) {
        return Err("XMAP14 combat progress is outside fixed5 bounds".into());
    }
    if template.matches("$VAL|0$").count() != 1 || template.replace("$VAL|0$", "").contains('$') {
        return Err("XMAP14 combat progress template is not exactly supported".into());
    }
    // The controller promotes fixed5 progress by x100 before binding VAL.
    // Clausewitz `|0` rounds this positive percentage to zero decimals.
    let percentage = (progress_fixed5 + 500) / 1_000;
    Ok(template.replacen("$VAL|0$", &percentage.to_string(), 1))
}

fn combat_font_content_paths(
    resolver: &GameFileResolver,
    admission: &ValidatedContentAdmission,
    assets: &CombatIndicatorAssetManifest,
    localisation_language: &str,
) -> Result<Vec<String>> {
    let mut paths = Vec::new();
    let font_names = [
        (&assets.gui_layouts.land_combat_mapicon, "combat_progress"),
        (&assets.gui_layouts.ongoing_combat_container, "count"),
        (&assets.gui_layouts.combat_results_container, "count"),
        (&assets.gui_layouts.tiny_unit_counter, "count"),
    ]
    .into_iter()
    .map(|(layout, control)| {
        layout
            .control(control)
            .and_then(|value| value.font.as_deref())
            .ok_or_else(|| {
                format!(
                    "XMAP14 combat font control {}.{control} has no FILE font",
                    layout.name
                )
                .into()
            })
    })
    .collect::<Result<BTreeSet<_>>>()?;
    for font_name in font_names {
        let binding = select_combat_font_binding(assets, font_name, localisation_language)?;
        let font_bases = if let Some(path) = &binding.path {
            vec![path.clone()]
        } else {
            binding.font_files.clone()
        };
        if font_bases.is_empty() {
            return Err(format!("XMAP14 {font_name} binding declares no font files").into());
        }
        for base in font_bases {
            let base = normalize_game_relative_path(base.trim_end_matches(".fnt"))
                .map_err(|error| format!("XMAP14 font base: {error}"))?;
            if !base.starts_with("gfx/fonts/") {
                return Err(format!("XMAP14 font base is outside gfx/fonts: {base}").into());
            }
            let descriptor = format!("{base}.fnt");
            let descriptor_key = admission
                .require_used_content(&descriptor)
                .map_err(|error| format!("XMAP14 font descriptor is not admitted: {error}"))?
                .clone();
            let descriptor_bytes = resolver.read_admitted_used_content(&descriptor_key)?;
            paths.push(descriptor);
            paths.extend(resolved_font_page_paths(
                resolver,
                &base,
                &latin1(&descriptor_bytes),
            )?);
        }
    }
    Ok(paths)
}

fn resolved_font_page_paths(
    resolver: &GameFileResolver,
    font_base: &str,
    descriptor: &str,
) -> Result<Vec<String>> {
    let font_name = font_base
        .strip_prefix("gfx/fonts/")
        .ok_or("XMAP14 font base is not normalized below gfx/fonts")?;
    let mut pages = Vec::new();
    for line in descriptor.lines() {
        let line = line.trim_start();
        if !line.starts_with("page ") {
            continue;
        }
        let page_id =
            font_descriptor_u32(line, "id").ok_or("XMAP14 bitmapfont page has no numeric id")?;
        let declared = font_descriptor_quoted(line, "file");
        let mut candidates = Vec::new();
        if let Some(declared) = declared {
            let declared = declared.replace('\\', "/");
            let explicit_vfs = declared
                .get(.."gfx/".len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("gfx/"));
            if !explicit_vfs {
                if let Some((dir, _)) = font_name.rsplit_once('/') {
                    candidates.push(format!("gfx/fonts/{dir}/{declared}"));
                }
            }
            candidates.push(if declared.contains('/') {
                declared
            } else {
                format!("gfx/fonts/{declared}")
            });
        }
        if page_id == 0 {
            candidates.push(format!("gfx/fonts/{font_name}.dds"));
            candidates.push(format!("gfx/fonts/{font_name}.tga"));
        }
        candidates.push(format!("gfx/fonts/{font_name}_{page_id}.dds"));
        candidates.push(format!("gfx/fonts/{font_name}_{page_id}.tga"));
        candidates.dedup();

        let selected = candidates
            .into_iter()
            .find_map(|candidate| match resolver.resolve(&candidate) {
                Ok(Some(_)) => Some(Ok(candidate)),
                Ok(None) => None,
                Err(error) => Some(Err(error)),
            })
            .transpose()?
            .ok_or_else(|| {
                format!("XMAP14 cannot resolve bitmapfont {font_name} page {page_id}")
            })?;
        pages.push((page_id, selected));
    }
    if pages.is_empty() {
        return Err(format!("XMAP14 bitmapfont {font_name} declares no pages").into());
    }
    pages.sort_by_key(|(page_id, _)| *page_id);
    for (expected, (page_id, _)) in pages.iter().enumerate() {
        if *page_id != expected as u32 {
            return Err(format!(
                "XMAP14 bitmapfont {font_name} page census is not contiguous at {page_id}"
            )
            .into());
        }
    }
    Ok(pages.into_iter().map(|(_, path)| path).collect())
}

fn font_descriptor_u32(line: &str, key: &str) -> Option<u32> {
    let marker = format!("{key}=");
    line.split_ascii_whitespace()
        .find_map(|token| token.strip_prefix(&marker)?.parse().ok())
}

fn font_descriptor_quoted(line: &str, key: &str) -> Option<String> {
    let marker = format!("{key}=\"");
    let rest = line.split_once(&marker)?.1;
    Some(rest.split_once('"')?.0.to_string())
}

fn parse_color4(source: &str, key: &str) -> Result<[f32; 4]> {
    for line in source.lines() {
        let code = line.split("--").next().unwrap_or(line);
        let Some(pos) = code.find(key) else {
            continue;
        };
        let end = pos + key.len();
        let bytes = code.as_bytes();
        let ident = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
        if (pos != 0 && ident(bytes[pos - 1])) || (end < bytes.len() && ident(bytes[end])) {
            continue;
        }
        let Some(rest) = code[end..].trim_start().strip_prefix('=') else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix('{') else {
            continue;
        };
        let Some(close) = rest.find('}') else {
            continue;
        };
        let values = rest[..close]
            .split(',')
            .map(|value| value.trim().parse::<f32>())
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if values.len() != 4 || !values.iter().all(|value| value.is_finite()) {
            return Err(format!("XMAP14 colour {key} is not a finite RGBA tuple").into());
        }
        return Ok([values[0], values[1], values[2], values[3]]);
    }
    Err(format!("XMAP14 missing graphics colour {key}").into())
}

fn ordered_identity_sha256(envelope: &Xmap14CombatEnvelope) -> String {
    let mut bytes = Vec::new();
    for identity in envelope
        .land
        .iter()
        .map(|row| &row.identity)
        .chain(envelope.naval.iter().map(|row| &row.identity))
        .chain(envelope.naval_results.iter().map(|row| &row.identity))
    {
        bytes.extend_from_slice(identity.canonical_id().as_bytes());
        bytes.push(0);
    }
    Sha256::hex(&bytes)
}

fn census_sha256(envelope: &Xmap14CombatEnvelope) -> Result<String> {
    Ok(Sha256::hex(&serde_json::to_vec(&(
        envelope.hour_index,
        envelope.capture_sequence,
        &envelope.land,
        &envelope.naval,
        &envelope.naval_results,
    ))?))
}

fn used_content_digest(content: &[UsedContentKey]) -> Result<String> {
    if content.is_empty() {
        return Err("XMAP14 effective-VFS content closure is empty".into());
    }
    Ok(Sha256::hex(&serde_json::to_vec(content)?))
}

fn prepared_digest(frame: &PreparedXmap14Frame) -> Result<String> {
    Ok(Sha256::hex(&serde_json::to_vec(&(
        &frame.schema,
        frame.hour_index,
        &frame.combat_indicator_assignment,
        &frame.ordered_used_content,
        &frame
            .indicators
            .iter()
            .map(prepared_row_digest)
            .collect::<Result<Vec<_>>>()?,
        &frame
            .indicators
            .iter()
            .map(|row| &row.ordered_draws)
            .collect::<Vec<_>>(),
        &frame.hit_targets,
        &frame.enumerator_witness_sha256,
        &frame.semantic_input_sha256,
        &frame.xmap13_census_sha256,
        &frame.xmap13_admission_sha256,
    ))?))
}

fn validate_prepared_draw(draw: &PreparedXmap14Draw) -> Result<()> {
    let finite_bits = |bits: u32| f32::from_bits(bits).is_finite();
    match draw {
        PreparedXmap14Draw::Sprite(draw) => {
            if draw.texture_file.trim().is_empty()
                || !draw
                    .positions_f32_bits
                    .iter()
                    .flatten()
                    .copied()
                    .all(finite_bits)
                || !draw.uv_f32_bits.iter().flatten().copied().all(finite_bits)
                || !draw
                    .modulation_rgba_f32_bits
                    .iter()
                    .copied()
                    .all(finite_bits)
            {
                return Err("XMAP14 prepared sprite draw is invalid".into());
            }
        }
        PreparedXmap14Draw::Progress(draw) => {
            let state = f32::from_bits(draw.current_state_f32_bits);
            if draw.texture_file1.trim().is_empty()
                || draw.texture_file2.trim().is_empty()
                || draw.shader_file.trim().is_empty()
                || validate_sha256(&draw.shader_sha256, "XMAP14 progress shader").is_err()
                || !(0.0..=1.0).contains(&state)
                || !draw
                    .positions_f32_bits
                    .iter()
                    .flatten()
                    .copied()
                    .all(finite_bits)
                || !draw.uv_f32_bits.iter().flatten().copied().all(finite_bits)
            {
                return Err("XMAP14 prepared progress draw is invalid".into());
            }
        }
        PreparedXmap14Draw::Text(draw) => {
            if draw.font_name.trim().is_empty()
                || canonical_localisation_language(&draw.font_language).is_err()
                || draw.text.is_empty()
                || !draw.box_min_f32_bits.iter().copied().all(finite_bits)
                || !draw.box_size_f32_bits.iter().copied().all(finite_bits)
                || draw
                    .box_size_f32_bits
                    .iter()
                    .copied()
                    .map(f32::from_bits)
                    .any(|value| value <= 0.0)
            {
                return Err("XMAP14 prepared text draw is invalid".into());
            }
        }
        PreparedXmap14Draw::Counter(draw) => {
            if !matches!(
                draw.grid_control_name.as_str(),
                "attacker_grid" | "defender_grid"
            ) || canonical_localisation_language(&draw.font_language).is_err()
                || draw.ordered_member_identities.is_empty()
                || draw.member_count != draw.ordered_member_identities.len() as u32
                || draw.ordered_primitives.is_empty()
                || !draw.origin_f32_bits.iter().copied().all(finite_bits)
            {
                return Err("XMAP14 prepared naval counter-grid draw is invalid".into());
            }
        }
    }
    Ok(())
}

fn prepared_row_digest(row: &PreparedXmap14Indicator) -> Result<String> {
    let kind = match row.kind {
        Xmap14PreparedKind::Land => 0,
        Xmap14PreparedKind::Naval => 1,
        Xmap14PreparedKind::NavalResult => 2,
    };
    let viewer_side = match row.viewer_side {
        Xmap14ViewerSide::First => 0,
        Xmap14ViewerSide::Second => 1,
    };
    let land_tint = row.land_tint.map(|value| match value {
        Xmap14LandTint::Winning => 0,
        Xmap14LandTint::Balanced => 1,
        Xmap14LandTint::Losing => 2,
    });
    let result_subtype = row.naval_result_subtype.as_ref().map(|value| match value {
        Xmap14NavalResultSubtype::NavalBattle => 0,
        Xmap14NavalResultSubtype::PortStrike => 1,
        Xmap14NavalResultSubtype::NavalStrike => 2,
        Xmap14NavalResultSubtype::Convoy => 3,
    });
    Ok(Sha256::hex(&serde_json::to_vec(&(
        (
            &row.identity,
            kind,
            row.location_province_id,
            row.world_anchor_f32_bits,
            row.projected_screen_f32_bits,
            viewer_side,
            row.progress_fixed5,
            row.progress_fraction_f32_bits,
            land_tint,
            row.land_tint_rgba_f32_bits,
        ),
        (
            row.bombardment_visible,
            row.air_frame,
            row.land_frame,
            row.direction_radians_f32_bits,
            row.naval_frame,
            row.naval_large_progress,
            row.naval_result_frame,
            result_subtype,
        ),
    ))?))
}

pub fn prepared_identity_sha256(frame: &PreparedXmap14Frame) -> String {
    let mut bytes = Vec::new();
    for indicator in &frame.indicators {
        bytes.extend_from_slice(indicator.identity.canonical_id().as_bytes());
        bytes.push(0);
    }
    Sha256::hex(&bytes)
}

/// Every XMAP00 input XMAP14 owns must appear in this hour's authority
/// projection, either as an owner-proven admission or as a typed gap — never
/// as neither, which would mean the projection silently dropped an owned
/// input. The returned list is the gap subset only: the foundation rejects a
/// `consumed_gap_ids` entry that is not an outstanding gap, and an admitted
/// input is not a gap to consume.
fn require_owned_inputs(
    gaps: &[crate::MapAuthorityGap],
    admitted: &[crate::MapAuthorityAdmittedInput],
) -> Result<Vec<String>> {
    let mut consumed_gap_ids = Vec::new();
    for owned in OWNED_GAPS {
        let is_gap = gaps.iter().any(|candidate| candidate.input_id == owned);
        let is_admitted = admitted
            .iter()
            .any(|candidate| candidate.input_id == owned);
        if is_gap && is_admitted {
            return Err(format!(
                "XMAP14 authority projection both admitted and gapped owned input {owned}"
            )
            .into());
        }
        if is_gap {
            consumed_gap_ids.push(owned.to_string());
        } else if !is_admitted {
            return Err(format!("XMAP14 authority projection omitted owned gap {owned}").into());
        }
    }
    Ok(consumed_gap_ids)
}

fn unavailable_output(
    hour: i64,
    consumed_gap_ids: Vec<String>,
    reason: &str,
) -> FoundationFeatureOutput {
    unavailable_output_with_gap(
        hour,
        consumed_gap_ids,
        "XMAP14.exact_combat_indicator_carrier_unavailable",
        reason,
    )
}

/// The same typed unavailability, under a NAMED gap.
///
/// The recording having no carrier this hour and this install being unable to
/// build the layer at all are two different absences, and a reader has to be
/// able to tell them apart from the receipt alone rather than by reading the
/// prose.
fn unavailable_output_with_gap(
    hour: i64,
    consumed_gap_ids: Vec<String>,
    gap_id: &str,
    reason: &str,
) -> FoundationFeatureOutput {
    let gap_id = gap_id.to_string();
    FoundationFeatureOutput {
        disposition: FamilyDisposition::Unavailable {
            gap: FoundationGap {
                witness_sha256: Sha256::hex(
                    format!("{gap_id}\n{hour}\n{hour}\n{reason}").as_bytes(),
                ),
                gap_id,
                first_unavailable_hour: hour,
                last_unavailable_hour: hour,
                reason: reason.into(),
            },
        },
        prepared_terrain: None,
        prepared_political: None,
        prepared_thin_borders: None,
        prepared_rivers_straits: None,
        prepared_water: None,
        prepared_lighting: None,
        prepared_weather: None,
        prepared_effects: None,
        prepared_units: None,
        prepared_combat: None,
        prepared_orders: None,
        prepared_rail_supply: None,
        prepared_map_modes: None,
        prepared_map_furniture: None,
        consumed_gap_ids,
    }
}

fn validate_sha256(value: &str, role: &str) -> Result<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("{role} is not SHA-256").into());
    }
    Ok(())
}

fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| char::from(*byte)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_content_manifest::ContentLayerKind;

    fn empty_prepared_frame() -> PreparedXmap14Frame {
        let assignment = AssignedStage::seal(
            "a".repeat(64),
            XMAP14_COMBAT_STAGE_NAME,
            XMAP14_COMBAT_OPERATION_NAME,
            13,
        )
        .unwrap();
        let content = UsedContentKey {
            effective_vfs_manifest_sha256: "b".repeat(64),
            normalized_game_relative_path: "interface/mapicons.gui".into(),
            winning_layer_ordinal: 0,
            layer_kind: ContentLayerKind::Base,
            layer_id: "base".into(),
            source_sha256: "c".repeat(64),
            payload_sha256: "d".repeat(64),
            payload_bytes: 1,
        };
        let mut frame = PreparedXmap14Frame {
            schema: XMAP14_PREPARED_SCHEMA.into(),
            hour_index: 42,
            combat_indicator_assignment: assignment,
            ordered_used_content: vec![content],
            indicators: Vec::new(),
            hit_targets: Vec::new(),
            enumerator_witness_sha256: "e".repeat(64),
            semantic_input_sha256: "f".repeat(64),
            xmap13_census_sha256: "1".repeat(64),
            xmap13_admission_sha256: "2".repeat(64),
            deterministic_sha256: String::new(),
        };
        frame.deterministic_sha256 = prepared_digest(&frame).unwrap();
        frame.validate().unwrap();
        frame
    }

    fn offscreen_indicator(identity: Xmap14IndicatorIdentity) -> PreparedXmap14Indicator {
        let kind = match identity {
            Xmap14IndicatorIdentity::Land { .. } => Xmap14PreparedKind::Land,
            Xmap14IndicatorIdentity::Naval { .. } => Xmap14PreparedKind::Naval,
            Xmap14IndicatorIdentity::NavalResult { .. } => Xmap14PreparedKind::NavalResult,
        };
        PreparedXmap14Indicator {
            identity,
            kind,
            location_province_id: 42,
            world_anchor_f32_bits: [0.0f32.to_bits(); 3],
            projected_screen_f32_bits: None,
            viewer_side: Xmap14ViewerSide::First,
            progress_fixed5: None,
            progress_fraction_f32_bits: None,
            land_tint: None,
            land_tint_rgba_f32_bits: None,
            bombardment_visible: false,
            air_frame: None,
            land_frame: None,
            direction_radians_f32_bits: None,
            naval_frame: None,
            naval_large_progress: None,
            naval_result_frame: None,
            naval_result_subtype: None,
            ordered_draws: Vec::new(),
        }
    }

    #[test]
    fn land_tint_uses_exact_strict_ten_percent_threshold() {
        assert_eq!(
            land_tint(110_000, 100_000, Xmap14ViewerSide::First).unwrap(),
            Xmap14LandTint::Balanced
        );
        assert_eq!(
            land_tint(110_001, 100_000, Xmap14ViewerSide::First).unwrap(),
            Xmap14LandTint::Winning
        );
        assert_eq!(
            land_tint(100_000, 110_001, Xmap14ViewerSide::First).unwrap(),
            Xmap14LandTint::Losing
        );
        assert_eq!(
            land_tint(110_001, 100_000, Xmap14ViewerSide::Second).unwrap(),
            Xmap14LandTint::Losing
        );
    }

    #[test]
    fn naval_frame_matches_exact_seven_counter_branch() {
        assert!(naval_convoy_frame(&[0, 0, 0, 0, 0, 1, 0], &[0; 7]));
        assert!(naval_convoy_frame(
            &[1, 0, 0, 0, 0, 1, 0],
            &[0, 0, 0, 1, 0, 0, 0]
        ));
        assert!(!naval_convoy_frame(
            &[1, 0, 0, 0, 0, 1, 0],
            &[1, 0, 0, 0, 0, 0, 0]
        ));
    }

    #[test]
    fn naval_viewer_side_uses_typed_country_pair_without_taskforces() {
        assert_eq!(
            viewer_side_for_country_pair("ENG", "ENG", "GER").unwrap(),
            Xmap14ViewerSide::First
        );
        assert_eq!(
            viewer_side_for_country_pair("GER", "ENG", "GER").unwrap(),
            Xmap14ViewerSide::Second
        );
        assert_eq!(
            viewer_side_for_country_pair("USA", "ENG", "GER").unwrap(),
            Xmap14ViewerSide::First
        );
        assert!(viewer_side_for_country_pair("ENG", "ENG", "ENG").is_err());
    }

    #[test]
    fn naval_participant_join_allows_exact_convoy_empty_side() {
        let task_force = Xmap13UnitIdentity::TaskForce {
            type_id: 1,
            stable_id: 9,
        };
        assert!(validate_unit_sides(&[task_force], &[], true).is_ok());
        assert!(validate_unit_sides(&[], &[], true).is_ok());
    }

    #[test]
    fn direction_is_atan2_delta_x_negative_delta_z() {
        let base = [0.0f32.to_bits(), 0.0f32.to_bits(), 0.0f32.to_bits()];
        let north = [0.0f32.to_bits(), 0.0f32.to_bits(), (-1.0f32).to_bits()];
        assert_eq!(
            direction_radians(base, north).unwrap().to_bits(),
            0.0f32.to_bits()
        );
        let east = [1.0f32.to_bits(), 0.0f32.to_bits(), 0.0f32.to_bits()];
        assert!(
            (direction_radians(base, east).unwrap() - std::f32::consts::FRAC_PI_2).abs() < 1.0e-6
        );
    }

    #[test]
    fn mapicon_alignment_uses_signed_half_away_rounding() {
        let size = crate::GuiSize {
            width: 35,
            height: 34,
        };
        assert_eq!(
            aligned_mapicon_origin([18.0, 18.0, 0.0], size).unwrap(),
            [1.0, 1.0]
        );
        assert_eq!(
            aligned_mapicon_origin([17.0, 16.0, 0.0], size).unwrap(),
            [-1.0, -1.0]
        );
    }

    #[test]
    fn mapicon_cutoff_is_aligned_rectangle_intersection() {
        let size = crate::GuiSize {
            width: 35,
            height: 34,
        };
        assert!(aligned_rect_intersects_viewport(
            [-34.0, -33.0],
            size,
            [1920, 1080]
        ));
        assert!(!aligned_rect_intersects_viewport(
            [-35.0, 0.0],
            size,
            [1920, 1080]
        ));
        assert!(aligned_rect_intersects_viewport(
            [1919.0, 1079.0],
            size,
            [1920, 1080]
        ));
        assert!(!aligned_rect_intersects_viewport(
            [1920.0, 0.0],
            size,
            [1920, 1080]
        ));
    }

    #[test]
    fn bitmapfont_page_tokens_are_exact() {
        let line = r#"page id=3 file="nested/hoi_16mbs_3.dds""#;
        assert_eq!(font_descriptor_u32(line, "id"), Some(3));
        assert_eq!(
            font_descriptor_quoted(line, "file").as_deref(),
            Some("nested/hoi_16mbs_3.dds")
        );
        assert_eq!(font_descriptor_u32("page file=\"x.dds\"", "id"), None);
    }

    #[test]
    fn combat_progress_uses_effective_localisation_template() {
        assert_eq!(
            format_combat_progress("progress $VAL|0$%", 49_499).unwrap(),
            "progress 49%"
        );
        assert_eq!(
            format_combat_progress("progress $VAL|0$%", 49_500).unwrap(),
            "progress 50%"
        );
        assert!(format_combat_progress("$VAL|1$", 50_000).is_err());
        assert!(format_combat_progress("$VAL|0$ $OTHER$", 50_000).is_err());
    }

    #[test]
    fn prepared_draw_texture_census_distinguishes_progress_pair() {
        let sprite = PreparedXmap14Draw::Sprite(PreparedXmap14SpriteDraw {
            texture_file: "gfx/a.dds".into(),
            frame_index: 0,
            positions_f32_bits: [[0; 2]; 4],
            uv_f32_bits: [[0; 2]; 4],
            modulation_rgba_f32_bits: [0; 4],
        });
        assert_eq!(prepared_draw_textures(&sprite), vec!["gfx/a.dds"]);

        let progress = PreparedXmap14Draw::Progress(PreparedXmap14ProgressDraw {
            texture_file1: "gfx/loose.dds".into(),
            texture_file2: "gfx/win.dds".into(),
            shader_file: "gfx/fx/progress.shader".into(),
            shader_sha256: "a".repeat(64),
            positions_f32_bits: [[0; 2]; 4],
            uv_f32_bits: [[0; 2]; 4],
            current_state_f32_bits: 0,
        });
        assert_eq!(
            prepared_draw_textures(&progress),
            vec!["gfx/loose.dds", "gfx/win.dds", "gfx/fx/progress.shader"]
        );
    }

    #[test]
    fn hit_target_uses_closed_finite_screen_bounds() {
        let target = PreparedXmap14HitTarget {
            identity: Xmap14IndicatorIdentity::Land {
                combat_id: 7,
                generation: 3,
            },
            min_screen_f32_bits: [10.0f32.to_bits(), 20.0f32.to_bits()],
            max_screen_f32_bits: [30.0f32.to_bits(), 40.0f32.to_bits()],
            depth_f32_bits: 0.25f32.to_bits(),
        };
        assert!(target.contains(10.0, 20.0));
        assert!(target.contains(30.0, 40.0));
        assert!(!target.contains(9.99, 30.0));
        assert!(!target.contains(f32::NAN, 30.0));
    }

    #[test]
    fn hit_test_selects_nearest_overlapping_combat_indicator() {
        let mut frame = empty_prepared_frame();
        let far = Xmap14IndicatorIdentity::Land {
            combat_id: 7,
            generation: 3,
        };
        let near = Xmap14IndicatorIdentity::Land {
            combat_id: 8,
            generation: 4,
        };
        frame.hit_targets = vec![
            PreparedXmap14HitTarget {
                identity: far,
                min_screen_f32_bits: [10.0f32.to_bits(), 20.0f32.to_bits()],
                max_screen_f32_bits: [30.0f32.to_bits(), 40.0f32.to_bits()],
                depth_f32_bits: 0.75f32.to_bits(),
            },
            PreparedXmap14HitTarget {
                identity: near.clone(),
                min_screen_f32_bits: [10.0f32.to_bits(), 20.0f32.to_bits()],
                max_screen_f32_bits: [30.0f32.to_bits(), 40.0f32.to_bits()],
                depth_f32_bits: 0.25f32.to_bits(),
            },
        ];
        assert_eq!(frame.hit_test(20.0, 30.0), Some(&near));
        assert_eq!(frame.hit_test(31.0, 30.0), None);
    }

    #[test]
    fn hit_test_prefers_later_combat_bucket_before_depth() {
        let mut frame = empty_prepared_frame();
        let near_naval = Xmap14IndicatorIdentity::Naval {
            combat_id: 7,
            generation: 3,
        };
        let far_land = Xmap14IndicatorIdentity::Land {
            combat_id: 8,
            generation: 4,
        };
        frame.hit_targets = vec![
            PreparedXmap14HitTarget {
                identity: near_naval,
                min_screen_f32_bits: [10.0f32.to_bits(), 20.0f32.to_bits()],
                max_screen_f32_bits: [30.0f32.to_bits(), 40.0f32.to_bits()],
                depth_f32_bits: 0.25f32.to_bits(),
            },
            PreparedXmap14HitTarget {
                identity: far_land.clone(),
                min_screen_f32_bits: [10.0f32.to_bits(), 20.0f32.to_bits()],
                max_screen_f32_bits: [30.0f32.to_bits(), 40.0f32.to_bits()],
                depth_f32_bits: 0.75f32.to_bits(),
            },
        ];
        assert_eq!(frame.hit_test(20.0, 30.0), Some(&far_land));
    }

    #[test]
    fn prepared_frame_rejects_non_engine_combat_bucket_order() {
        let mut frame = empty_prepared_frame();
        frame.indicators = vec![
            offscreen_indicator(Xmap14IndicatorIdentity::Land {
                combat_id: 3,
                generation: 1,
            }),
            offscreen_indicator(Xmap14IndicatorIdentity::NavalResult {
                result_id: 2,
                generation: 1,
                create_sequence: 1,
            }),
            offscreen_indicator(Xmap14IndicatorIdentity::Naval {
                combat_id: 1,
                generation: 1,
            }),
        ];
        frame.deterministic_sha256 = prepared_digest(&frame).unwrap();
        assert!(frame
            .validate()
            .unwrap_err()
            .to_string()
            .contains("exact engine bucket order"));

        frame
            .indicators
            .sort_by_key(|row| row.identity.map_icon_bucket_ordinal());
        frame.deterministic_sha256 = prepared_digest(&frame).unwrap();
        frame.validate().unwrap();
        assert_eq!(
            frame
                .indicators
                .iter()
                .map(|row| row.identity.map_icon_bucket_ordinal())
                .collect::<Vec<_>>(),
            vec![9, 10, 11]
        );
    }

    #[test]
    fn typed_census_rejects_unavailable_or_non_land_rows_before_counting() {
        let row = CombatRow {
            combat_id: Leaf::Measured(7),
            generation: Leaf::Measured(3),
            kind: Leaf::Unobtained,
            location_province: Leaf::Measured(42),
            attackers: Leaf::Measured(Vec::new()),
            defenders: Leaf::Measured(Vec::new()),
            combat_width: Leaf::Unobtained,
            terrain: Leaf::Unobtained,
            progress: Leaf::Unobtained,
            detail: Leaf::Unobtained,
        };
        let error = validate_typed_land_census(&[row], &[]).unwrap_err();
        assert!(error.to_string().contains("unavailable or non-land kind"));
    }

    #[test]
    fn typed_land_identity_uses_top_level_d21_generation_without_d55_detail() {
        let row = CombatRow {
            combat_id: Leaf::Measured(7),
            generation: Leaf::Measured(3),
            kind: Leaf::Measured("land".into()),
            location_province: Leaf::Measured(42),
            attackers: Leaf::Measured(Vec::new()),
            defenders: Leaf::Measured(Vec::new()),
            combat_width: Leaf::Unobtained,
            terrain: Leaf::Unobtained,
            progress: Leaf::Unobtained,
            detail: Leaf::Unobtained,
        };
        let carrier = Xmap14LandCarrier {
            identity: Xmap14IndicatorIdentity::Land {
                combat_id: 7,
                generation: 3,
            },
            location_province_id: 42,
            combat_base_point_f32_bits: [0; 3],
            first_live_attacker_stack_point_f32_bits: [0; 3],
            progress_fixed5: 0,
            attacker_comparison_fixed: 0,
            defender_comparison_fixed: 0,
            attacker_side_flags: 0,
            defender_side_flags: 0,
            side_flags_sha256: "a".repeat(64),
            frame_selector: 0,
            attackers: Vec::new(),
            defenders: Vec::new(),
            active_phase: true,
        };
        validate_typed_land_census(&[row], &[carrier]).unwrap();
    }

    #[test]
    fn authenticated_empty_execution_receipt_binds_frame_and_target() {
        let frame = empty_prepared_frame();
        let target = "3".repeat(64);
        let assignment = &frame.combat_indicator_assignment;
        let receipt = Xmap14ExecutionReceipt {
            schema: XMAP14_NATIVE_RECEIPT_SCHEMA.into(),
            hour_index: frame.hour_index,
            executor_target_sha256: target.clone(),
            assigned_topology_sha256: assignment.topology_sha256.clone(),
            assigned_stage_id: assignment.stage_name.clone(),
            assigned_operation_id: assignment.operation_name.clone(),
            assigned_ordinal: assignment.assigned_ordinal,
            assigned_assignment_sha256: assignment.assignment_sha256.clone(),
            prepared_frame_sha256: frame.deterministic_sha256.clone(),
            hit_targets_sha256: Sha256::hex(&serde_json::to_vec(&frame.hit_targets).unwrap()),
            draw: Xmap14NativeDrawClosure::AuthenticatedEmpty {
                enumerator_witness_sha256: frame.enumerator_witness_sha256.clone(),
            },
            deterministic_sha256: String::new(),
        }
        .seal()
        .unwrap();
        receipt.validate_for(&frame, &target).unwrap();
        assert!(receipt.validate_for(&frame, &"4".repeat(64)).is_err());
    }

    #[test]
    fn authenticated_culled_receipt_closes_a_nonempty_offscreen_census() {
        let mut frame = empty_prepared_frame();
        frame.indicators.push(PreparedXmap14Indicator {
            identity: Xmap14IndicatorIdentity::Land {
                combat_id: 9,
                generation: 2,
            },
            kind: Xmap14PreparedKind::Land,
            location_province_id: 42,
            world_anchor_f32_bits: [0.0f32.to_bits(); 3],
            projected_screen_f32_bits: None,
            viewer_side: Xmap14ViewerSide::First,
            progress_fixed5: Some(50_000),
            progress_fraction_f32_bits: Some(0.5f32.to_bits()),
            land_tint: Some(Xmap14LandTint::Balanced),
            land_tint_rgba_f32_bits: Some([1.0f32.to_bits(); 4]),
            bombardment_visible: false,
            air_frame: None,
            land_frame: Some(1),
            direction_radians_f32_bits: Some(0.0f32.to_bits()),
            naval_frame: None,
            naval_large_progress: None,
            naval_result_frame: None,
            naval_result_subtype: None,
            ordered_draws: Vec::new(),
        });
        frame.deterministic_sha256 = prepared_digest(&frame).unwrap();
        frame.validate().unwrap();

        let target = "3".repeat(64);
        let assignment = &frame.combat_indicator_assignment;
        let receipt = Xmap14ExecutionReceipt {
            schema: XMAP14_NATIVE_RECEIPT_SCHEMA.into(),
            hour_index: frame.hour_index,
            executor_target_sha256: target.clone(),
            assigned_topology_sha256: assignment.topology_sha256.clone(),
            assigned_stage_id: assignment.stage_name.clone(),
            assigned_operation_id: assignment.operation_name.clone(),
            assigned_ordinal: assignment.assigned_ordinal,
            assigned_assignment_sha256: assignment.assignment_sha256.clone(),
            prepared_frame_sha256: frame.deterministic_sha256.clone(),
            hit_targets_sha256: Sha256::hex(&serde_json::to_vec(&frame.hit_targets).unwrap()),
            draw: Xmap14NativeDrawClosure::AuthenticatedCulled {
                enumerator_witness_sha256: frame.enumerator_witness_sha256.clone(),
                indicator_count: 1,
                ordered_identity_sha256: frame.ordered_identity_sha256(),
            },
            deterministic_sha256: String::new(),
        }
        .seal()
        .unwrap();
        receipt.validate_for(&frame, &target).unwrap();
    }

    #[test]
    fn centered_leaf_anchor_uses_the_immediate_parent_extent() {
        let mut control = crate::GuiControlLayout {
            kind: "iconType".into(),
            name: "bg_btn".into(),
            source_file: "interface/mapicons.gui".into(),
            source_line: 1,
            sprite_type: Some("GFX_mapicon_naval_combat".into()),
            quad_texture_sprite: None,
            parent_size: Some(crate::GuiSize {
                width: 33,
                height: 43,
            }),
            position: Some(crate::GuiPoint { x: 1, y: -30 }),
            effective_position_f32_bits: Some([1.0f32.to_bits(), (-30.0f32).to_bits()]),
            size: None,
            slot_size: None,
            max_width: None,
            max_height: None,
            font: None,
            format: None,
            orientation: Some("center".into()),
            max_slots_vertical: None,
            rotation: None,
            scale: None,
            frame: None,
            center_position: Some(true),
        };
        assert_eq!(control_anchor(&control, [0.0, 0.0]).unwrap(), [17.5, -8.5]);
        let size = crate::GuiSize {
            width: 8,
            height: 6,
        };
        assert_eq!(
            control_rect_min(&control, [0.0, 0.0], size).unwrap(),
            [13.5, -11.5]
        );
        control.center_position = Some(false);
        assert_eq!(
            control_rect_min(&control, [0.0, 0.0], size).unwrap(),
            [17.5, -8.5]
        );
    }

    #[test]
    fn progress_shader_validation_binds_the_complete_program() {
        let supported = SUPPORTED_PROGRESS_SHADER_SOURCE;
        validate_supported_progress_shader(supported.as_bytes(), "supported.shader").unwrap();
        assert!(validate_supported_progress_shader(
            supported.replace("<=", ">=").as_bytes(),
            "changed.shader"
        )
        .is_err());
        assert!(validate_supported_progress_shader(
            format!("/* {supported} */").as_bytes(),
            "comment-only.shader"
        )
        .is_err());
        assert!(validate_supported_progress_shader(
            format!("{supported}\n{supported}").as_bytes(),
            "duplicate.shader"
        )
        .is_err());
        assert!(validate_supported_progress_shader(
            supported
                .replace(
                    "return tex2D(TextureTwo, v.vTexCoord0.xy);",
                    "return tex2D(TextureTwo, v.vTexCoord0.xy) * float4(1, 0, 0, 1);",
                )
                .as_bytes(),
            "extra-live-tint.shader"
        )
        .is_err());
        assert!(validate_supported_progress_shader(
            format!("{supported}\nfloat4 ExtraTint;").as_bytes(),
            "extra-live-declaration.shader"
        )
        .is_err());
        assert!(validate_supported_progress_shader(
            supported
                .replace("const VS_INPUT v", "constVS_INPUT v")
                .as_bytes(),
            "merged-identifiers.shader"
        )
        .is_err());
        assert!(validate_supported_progress_shader(
            supported.replace("\"Point\"", "\"Po int\"").as_bytes(),
            "changed-quoted-token.shader"
        )
        .is_err());
        let mut non_ascii = supported.as_bytes().to_vec();
        non_ascii.push(0xff);
        assert!(validate_supported_progress_shader(&non_ascii, "non-ascii.shader").is_err());
        validate_supported_progress_shader(
            supported
                .replace("const VS_INPUT v", "const/**/VS_INPUT v")
                .as_bytes(),
            "inline-comment.shader",
        )
        .unwrap();
        validate_supported_progress_shader(
            format!("/* harmless comment */\n{supported}\n// harmless tail\n").as_bytes(),
            "comment-and-whitespace-only.shader",
        )
        .unwrap();
    }

    fn gap(input_id: &str) -> crate::MapAuthorityGap {
        crate::MapAuthorityGap {
            input_id: input_id.into(),
            disposition: crate::MapAuthorityRequirementDisposition::Unavailable,
            reason: "fixture blocker".into(),
        }
    }

    fn admitted(input_id: &str) -> crate::MapAuthorityAdmittedInput {
        let proven = crate::map_authority_owner_proven_admission(input_id).unwrap();
        crate::MapAuthorityAdmittedInput {
            input_id: input_id.into(),
            owner_packet: proven.owner_packet.into(),
            payload_schema: proven.payload_schema.into(),
            cadence: proven.cadence.into(),
            identity_order: proven.identity_order.into(),
        }
    }

    #[test]
    fn owned_inputs_must_be_admitted_xor_gapped_and_only_gaps_are_consumed() {
        let all_gaps: Vec<_> = OWNED_GAPS.iter().map(|id| gap(id)).collect();
        assert_eq!(
            require_owned_inputs(&all_gaps, &[]).unwrap(),
            OWNED_GAPS.iter().map(|id| id.to_string()).collect::<Vec<_>>()
        );

        let all_admitted: Vec<_> = OWNED_GAPS.iter().map(|id| admitted(id)).collect();
        assert!(require_owned_inputs(&[], &all_admitted).unwrap().is_empty());

        assert_eq!(
            require_owned_inputs(
                &[gap("combat.land_terminal_detail")],
                &[admitted("combat.land_active_epoch"), admitted("combat.naval_air_epoch")],
            )
            .unwrap(),
            vec!["combat.land_terminal_detail".to_string()]
        );

        assert!(require_owned_inputs(
            &[gap("combat.land_active_epoch"), gap("combat.land_terminal_detail")],
            &[],
        )
        .unwrap_err()
        .to_string()
        .contains("omitted owned gap combat.naval_air_epoch"));

        assert!(require_owned_inputs(&all_gaps, &all_admitted)
            .unwrap_err()
            .to_string()
            .contains("both admitted and gapped"));
    }

    /// A DARK MAP LAYER IS EXCLUDED AND NAMED, NEVER A BLANK FRAME.
    ///
    /// Building this adapter reads the install's authored GFX. Any refusal in
    /// there used to travel out of `Xmap14Adapter::new` through the `?` in
    /// `production_foundation::product_feature_adapters_with_assigned_stage_schedule`
    /// and destroy the whole adapter graph, so one sprite cost the entire
    /// picture. Measured on capture t83: shipped vanilla authors
    /// `GFX_land_unit_combat_progressbar` (`interface/unitview.gfx:770`) with no
    /// `horizontal` key -- as do 62 of the install's 105 `progressbartype`
    /// blocks -- and the frame chain reached D3D11 and the whole localisation
    /// corpus and then produced no pixel, no BMP and no receipt at all.
    ///
    /// `new_or_unavailable` is the ONLY constructor and cannot refuse; it
    /// carries the refusal text verbatim so the receipt names the sprite and
    /// the reason rather than merely counting one fewer layer.
    ///
    /// This test covers the layer's own contract. That an unavailable layer
    /// then survives the whole frame is a different claim, proved where it
    /// lives -- `replay_app`'s
    /// `xmap22_composition_receipt_is_content_bound_and_deny_unknown`, which
    /// seals a receipt carrying an unavailable combat family and refuses a
    /// tampered one, and covers the compositor's receipt-or-gap choice through
    /// `xmap22_execution_witness_or_named_gap`. What no test covers is the GPU
    /// path around that choice.
    ///
    /// kills: dropping the reason from the gap (the reason assertion fails);
    /// reporting the content-unavailable case under the carrier gap id (the
    /// last assertion fails, and a reader could no longer tell "the recording
    /// has no carrier this hour" from "this install cannot build the layer at
    /// all"); giving the unavailable layer a prepared payload (the
    /// `prepared_combat` assertion fails).
    #[test]
    fn an_install_that_cannot_build_the_combat_layer_costs_the_layer_and_names_it() {
        // There IS no fallible constructor any more, so the product graph
        // cannot register one: `new_or_unavailable` returns `Self`, and the
        // type system carries what a source-string assertion used to guess at.

        // The exact refusal shipped vanilla produces, measured on t83.
        let reason =
            "XMAP14 progress bar GFX_land_unit_combat_progressbar is not authored horizontal"
                .to_string();
        let adapter = Xmap14Adapter {
            state: Xmap14State::Unavailable {
                reason: reason.clone(),
            },
        };
        assert_eq!(
            adapter.family(),
            MapVisualFamily::CombatMarkers,
            "the family still has a row, so the receipt says WHICH layer is missing"
        );

        // The disposition an unavailable layer produces is the CONTENT gap,
        // carrying the refusal verbatim.
        let output = unavailable_output_with_gap(
            42,
            Vec::new(),
            "XMAP14.combat_indicator_content_unavailable",
            &reason,
        );
        let FamilyDisposition::Unavailable { gap } = &output.disposition else {
            panic!("the content-unavailable layer must be typed unavailable, never an error");
        };
        assert_eq!(gap.gap_id, "XMAP14.combat_indicator_content_unavailable");
        assert_eq!(gap.reason, reason, "a reader is entitled to the reason");
        assert_eq!(gap.first_unavailable_hour, 42);
        assert_eq!(gap.last_unavailable_hour, 42);
        assert!(output.prepared_combat.is_none(), "no payload is invented");

        // The recording having no carrier THIS HOUR and this install being
        // unable to build the layer AT ALL are different facts.
        let carrier = unavailable_output(42, Vec::new(), "no carrier this hour");
        let FamilyDisposition::Unavailable { gap: carrier_gap } = &carrier.disposition else {
            panic!("the carrier-absent layer is typed unavailable too");
        };
        assert_ne!(
            carrier_gap.gap_id, gap.gap_id,
            "two different absences must not share one gap id"
        );
    }
}
