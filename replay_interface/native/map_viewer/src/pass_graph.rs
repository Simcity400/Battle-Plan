use crate::*;
#[cfg(windows)]
use winapi::um::d3dcommon::D3D11_SRV_DIMENSION_TEXTURE2D;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProductionMapPassKind {
    Terrain,
    Water,
    Rivers,
    Borders,
    Trees,
    Cities,
    Overlays,
}

impl ProductionMapPassKind {
    pub(crate) fn capture_moment(self) -> &'static str {
        match self {
            Self::Terrain => TERRAIN_CAPTURE_MOMENT,
            Self::Water => WATER_CAPTURE_MOMENT,
            Self::Rivers => RIVERS_CAPTURE_MOMENT,
            Self::Borders => BORDERS_CAPTURE_MOMENT,
            Self::Trees => TREES_CAPTURE_MOMENT,
            Self::Cities => CITIES_CAPTURE_MOMENT,
            Self::Overlays => OVERLAYS_CAPTURE_MOMENT,
        }
    }

    pub(crate) fn from_capture_moment(capture_moment: &str) -> Option<Self> {
        match capture_moment {
            TERRAIN_CAPTURE_MOMENT => Some(Self::Terrain),
            WATER_CAPTURE_MOMENT => Some(Self::Water),
            RIVERS_CAPTURE_MOMENT => Some(Self::Rivers),
            BORDERS_CAPTURE_MOMENT => Some(Self::Borders),
            TREES_CAPTURE_MOMENT => Some(Self::Trees),
            CITIES_CAPTURE_MOMENT => Some(Self::Cities),
            OVERLAYS_CAPTURE_MOMENT => Some(Self::Overlays),
            _ => None,
        }
    }

    pub(crate) fn pass_label(self) -> &'static str {
        match self {
            Self::Terrain => "terrain",
            Self::Water => "water",
            Self::Rivers => "rivers",
            Self::Borders => "borders",
            Self::Trees => "trees",
            Self::Cities => "cities",
            Self::Overlays => "overlays",
        }
    }

    pub(crate) fn blocked_until_captured_message(self) -> String {
        match self {
            Self::Terrain => {
                "terrain is the first app-owned production map pass and must be ready".to_string()
            }
            Self::Water => {
                "water must consume the app-owned terrain color/depth targets after terrain parity is validated; do not replay it as an isolated captured pass".to_string()
            }
            Self::Rivers => {
                "rivers require a named the game D3D11 river-pass bundle with generated RiverData, shader bytecode/reflection, draw state, and resources; do not render map/rivers.bmp as a screen overlay".to_string()
            }
            Self::Borders => {
                "borders require named the game D3D11 border/gradient-border pass bundles with generated channels or mesh data; do not invent fake province outlines".to_string()
            }
            Self::Trees => {
                "trees require a named the game D3D11 tree pass bundle or proven generated scatter data from the game; do not scatter trees directly from map/trees.bmp".to_string()
            }
            Self::Cities => {
                "cities require a named the game D3D11 city pass bundle or proven generated placement/entity data from the game; do not draw city markers from map/cities.bmp".to_string()
            }
            Self::Overlays => {
                "overlays require named the game D3D11 overlay/map-mode pass bundles with generated runtime textures and draw state; do not approximate map modes from static files".to_string()
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProductionMapPassStatus {
    Ready,
    Blocked,
}

/// Production disposition of each close-zoom law module.  A source-level law
/// is not a draw authorization: `PixelEvidenceBlocked` means the exact input
/// seam exists, but no authenticated engine/replay pixel pair is pinned to the
/// execution receipt yet.  `InputBlocked` means an exact runtime carrier is
/// still absent.  Neither state may enable a draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProductionCloseZoomLawStatus {
    InputBlocked,
    PixelEvidenceBlocked,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductionCloseZoomLawPlan {
    pub module: &'static str,
    pub status: ProductionCloseZoomLawStatus,
    pub blockers: Vec<&'static str>,
}

fn production_close_zoom_law_plan() -> Vec<ProductionCloseZoomLawPlan> {
    use ProductionCloseZoomLawStatus::{InputBlocked, PixelEvidenceBlocked};

    vec![
        ProductionCloseZoomLawPlan {
            module: "shadow_law",
            status: InputBlocked,
            blockers: vec![
                "S1 authenticated per-frame shadow target producer/last-write chain is absent; law constants alone do not authorize caster selection, ordering, mutable transforms, blur ownership, or pixels",
            ],
        },
        ProductionCloseZoomLawPlan {
            module: "building_files",
            status: InputBlocked,
            blockers: vec![
                "generic building_files has no complete per-hour object/state/LOD/material-variant carrier; preview_all candidates are diagnostic and must never become production instances",
            ],
        },
        ProductionCloseZoomLawPlan {
            module: "rail_files",
            status: InputBlocked,
            blockers: vec![
                "the replay has no ordered CRailwayGfx province/level pairs, active byte, or supply-hub sprite frame; centroid polylines are approximate and forbidden in production",
            ],
        },
        ProductionCloseZoomLawPlan {
            module: "occupation_stripe_law",
            status: PixelEvidenceBlocked,
            blockers: vec![
                "the captured shader and byte-native ProvinceSecondaryColorMap seam are exact, but no authenticated raw portable producer/consumer observation plus receipt-bound engine/replay pixel golden is admitted",
            ],
        },
        ProductionCloseZoomLawPlan {
            module: "tree_instance_synthesis",
            status: InputBlocked,
            blockers: vec![
                "the retained 4,781-cell scope has an exact pointer-free 13-buffer/three-pass visible model/LOD carrier and executes through authenticated static operations; complete-pool synthesis remains blocked on the other 95,275 nonzero cells, and collapsing batches to one mesh draw is forbidden",
            ],
        },
        ProductionCloseZoomLawPlan {
            module: "camera_interp_law",
            status: InputBlocked,
            blockers: vec![
                "dt normalization and the exact height-to-zoom multiplier remain unresolved; production must consume measured camera projection state instead of applying the partial interpolation helper",
            ],
        },
        ProductionCloseZoomLawPlan {
            module: "snow_mud_feed",
            status: PixelEvidenceBlocked,
            blockers: vec![
                "the exact CPU pre-upload packet and replay texture readback/replacement route is pointer-free, but selected-Present SnowMudFade authority, a distinct-session raw H/H2 join, and a receipt-bound engine/replay pixel golden are not admitted",
            ],
        },
    ]
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductionMapTargetPlan {
    pub width: u32,
    pub height: u32,
    pub color_format: u32,
    pub depth_format: u32,
    pub sample_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductionMapPassPlan {
    pub kind: ProductionMapPassKind,
    pub capture_moment: &'static str,
    pub status: ProductionMapPassStatus,
    pub depends_on: Vec<ProductionMapPassKind>,
    pub blockers: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProductionMapPassGraph {
    pub camera: ProductionMapCamera,
    pub target: ProductionMapTargetPlan,
    pub passes: Vec<ProductionMapPassPlan>,
    pub close_zoom_laws: Vec<ProductionCloseZoomLawPlan>,
}

pub(crate) fn production_blocked_map_pass(
    kind: ProductionMapPassKind,
    depends_on: Vec<ProductionMapPassKind>,
) -> ProductionMapPassPlan {
    ProductionMapPassPlan {
        kind,
        capture_moment: kind.capture_moment(),
        status: ProductionMapPassStatus::Blocked,
        depends_on,
        blockers: vec![kind.blocked_until_captured_message()],
    }
}

pub(crate) fn production_pass_plan(
    terrain_capture_moment: &'static str,
) -> Vec<ProductionMapPassPlan> {
    vec![
        ProductionMapPassPlan {
            kind: ProductionMapPassKind::Terrain,
            capture_moment: terrain_capture_moment,
            status: ProductionMapPassStatus::Ready,
            depends_on: Vec::new(),
            blockers: Vec::new(),
        },
        production_blocked_map_pass(
            ProductionMapPassKind::Water,
            vec![ProductionMapPassKind::Terrain],
        ),
        production_blocked_map_pass(
            ProductionMapPassKind::Rivers,
            vec![ProductionMapPassKind::Water],
        ),
        production_blocked_map_pass(
            ProductionMapPassKind::Borders,
            vec![ProductionMapPassKind::Rivers],
        ),
        production_blocked_map_pass(
            ProductionMapPassKind::Trees,
            vec![ProductionMapPassKind::Rivers],
        ),
        production_blocked_map_pass(
            ProductionMapPassKind::Cities,
            vec![ProductionMapPassKind::Trees],
        ),
        production_blocked_map_pass(
            ProductionMapPassKind::Overlays,
            vec![ProductionMapPassKind::Cities],
        ),
    ]
}

pub(crate) fn water_refraction_app_owned_terrain_color_srv_desc(
    frame: &MapFrameRuntimeBundle,
    terrain_color_desc: &MapFrameTexture2DDesc,
) -> Result<Option<MapFrameShaderResourceViewDesc>> {
    if frame.capture_moment != WATER_CAPTURE_MOMENT {
        return Ok(None);
    }
    let mut expected = None;
    for draw in &frame.draws {
        let desc = validate_water_refraction_draw_source(draw, terrain_color_desc)?;
        match expected.as_ref() {
            None => expected = Some(desc),
            Some(reference) if reference != &desc => {
                return Err(format!(
                    "production water renderer blocked: map-frame draw {} {WATER_REFRACTION_RESOURCE_SYMBOL} SRV descriptor differs from an earlier water draw",
                    draw.draw_index
                )
                .into())
            }
            Some(_) => {}
        }
    }
    expected.map(Some).ok_or_else(|| {
        "production water renderer blocked: water pass has no draw sequence proving WaterRefraction"
            .into()
    })
}

pub(crate) fn validate_water_refraction_draw_source(
    draw: &MapFrameDraw,
    terrain_color_desc: &MapFrameTexture2DDesc,
) -> Result<MapFrameShaderResourceViewDesc> {
    let slot = draw
        .shader
        .reflection_resources
        .get(WATER_REFRACTION_RESOURCE_SYMBOL)
        .copied()
        .ok_or_else(|| {
            format!(
                "production water renderer blocked: draw {} active pixel shader reflection does not declare {WATER_REFRACTION_RESOURCE_SYMBOL}",
                draw.draw_index
            )
        })?;
    let resource = draw
        .shader_resources
        .iter()
        .find(|resource| resource.stage == "PS" && resource.slot == slot)
        .ok_or_else(|| {
            format!(
                "production water renderer blocked: draw {} declares {WATER_REFRACTION_RESOURCE_SYMBOL} at PS slot {slot}, but no SRV binding was captured",
                draw.draw_index
            )
        })?;
    if resource.declared_name.as_deref() != Some(WATER_REFRACTION_RESOURCE_SYMBOL) {
        return Err(format!(
            "production water renderer blocked: draw {} PS slot {slot} is captured as {:?}, expected {WATER_REFRACTION_RESOURCE_SYMBOL}",
            draw.draw_index, resource.declared_name
        )
        .into());
    }
    if resource.is_null {
        return Err(format!(
            "production water renderer blocked: draw {} {WATER_REFRACTION_RESOURCE_SYMBOL} PS slot {slot} is null; app-owned terrain color cannot be bound without captured shader evidence",
            draw.draw_index
        )
        .into());
    }
    let view = resource.view_desc.as_ref().ok_or_else(|| {
        format!(
            "production water renderer blocked: draw {} {WATER_REFRACTION_RESOURCE_SYMBOL} PS slot {slot} has no captured SRV descriptor",
            draw.draw_index
        )
    })?;
    if view.view_dimension != D3D11_SRV_DIMENSION_TEXTURE2D
        || view.most_detailed_mip != 0
        || view.mip_levels != 1
        || view.first_array_slice != 0
        || view.array_size > 1
        || view.num_cubes != 0
    {
        return Err(format!(
            "production water renderer blocked: draw {} {WATER_REFRACTION_RESOURCE_SYMBOL} must be a single Texture2D mip0 SRV for the app-owned terrain color target",
            draw.draw_index
        )
        .into());
    }
    if view.format != resource.dxgi_format {
        return Err(format!(
            "production water renderer blocked: draw {} {WATER_REFRACTION_RESOURCE_SYMBOL} SRV format {} does not match captured resource format {}",
            draw.draw_index, view.format, resource.dxgi_format
        )
        .into());
    }
    if resource.width != terrain_color_desc.width || resource.height != terrain_color_desc.height {
        return Err(format!(
            "production water renderer blocked: draw {} {WATER_REFRACTION_RESOURCE_SYMBOL} {}x{} format {} cannot source app-owned terrain color {}x{} converted to format {}",
            draw.draw_index,
            resource.width,
            resource.height,
            resource.dxgi_format,
            terrain_color_desc.width,
            terrain_color_desc.height,
            resource.dxgi_format
        )
        .into());
    }
    if resource.mip_count != 1 || resource.array_size != 1 || resource.sample_count != 1 {
        return Err(format!(
            "production water renderer blocked: draw {} {WATER_REFRACTION_RESOURCE_SYMBOL} requires a resolved single-mip Texture2D SRV, got mip_count {} array_size {} sample_count {}",
            draw.draw_index, resource.mip_count, resource.array_size, resource.sample_count
        )
        .into());
    }
    if resource.bind_flags & D3D11_BIND_SHADER_RESOURCE == 0 {
        return Err(format!(
            "production water renderer blocked: draw {} {WATER_REFRACTION_RESOURCE_SYMBOL} PS slot {slot} is missing D3D11_BIND_SHADER_RESOURCE evidence",
            draw.draw_index
        )
        .into());
    }
    Ok(view.clone())
}

impl ProductionMapPassGraph {
    pub fn from_terrain_capture(
        terrain_bundle: &NativeD3d11TerrainBundle,
        camera: ProductionMapCamera,
    ) -> Result<Self> {
        camera.validate()?;
        if terrain_bundle.frame.capture_moment != TERRAIN_CAPTURE_MOMENT {
            return Err(format!(
                "production map renderer blocked: first production pass must be {TERRAIN_CAPTURE_MOMENT}, got {}",
                terrain_bundle.frame.capture_moment
            )
            .into());
        }
        let (color, depth) = production_map_target_evidence(&terrain_bundle.frame)?;
        if color.desc.sample_count != depth.desc.sample_count {
            return Err(format!(
                "production map renderer blocked: terrain color target sample count {} does not match depth target sample count {}",
                color.desc.sample_count, depth.desc.sample_count
            )
            .into());
        }
        let target = ProductionMapTargetPlan {
            width: color.desc.width,
            height: color.desc.height,
            color_format: color.desc.format,
            depth_format: depth.desc.format,
            sample_count: color.desc.sample_count,
        };
        Ok(Self {
            camera,
            target,
            passes: production_pass_plan(ProductionMapPassKind::Terrain.capture_moment()),
            close_zoom_laws: production_close_zoom_law_plan(),
        })
    }

    pub fn from_terrain_chunk_library(
        terrain_library: &NativeD3d11TerrainChunkLibrary,
        camera: ProductionMapCamera,
    ) -> Result<Self> {
        camera.validate()?;
        let frame = terrain_library.target_evidence_frame()?;
        let blockers = native_d3d11_replay_payload_blockers(&frame, false)
            .into_iter()
            .filter(|blocker| !blocker.contains("post-pass output-merger"))
            .collect::<Vec<_>>();
        if !blockers.is_empty() {
            return Err(format!(
                "production map renderer blocked: terrain chunk library native D3D11 replay payload is incomplete: {}",
                blockers.join("; ")
            )
            .into());
        }
        let (color, depth) = production_map_target_evidence(&frame)?;
        if color.desc.sample_count != depth.desc.sample_count {
            return Err(format!(
                "production map renderer blocked: terrain chunk library color target sample count {} does not match depth target sample count {}",
                color.desc.sample_count, depth.desc.sample_count
            )
            .into());
        }
        if terrain_library.coverage_fraction() < 1.0 {
            return Err(format!(
                "production map renderer blocked: terrain chunk library coverage is {:.6}; rendering would contain {} captured-evidence holes",
                terrain_library.coverage_fraction(),
                terrain_library.holes().len()
            )
            .into());
        }
        let target = ProductionMapTargetPlan {
            width: color.desc.width,
            height: color.desc.height,
            color_format: color.desc.format,
            depth_format: depth.desc.format,
            sample_count: color.desc.sample_count,
        };
        Ok(Self {
            camera,
            target,
            passes: production_pass_plan(TERRAIN_CHUNK_LIBRARY_MOMENT),
            close_zoom_laws: production_close_zoom_law_plan(),
        })
    }

    pub fn pass(&self, kind: ProductionMapPassKind) -> Option<&ProductionMapPassPlan> {
        self.passes.iter().find(|pass| pass.kind == kind)
    }

    pub fn close_zoom_law(&self, module: &str) -> Option<&ProductionCloseZoomLawPlan> {
        self.close_zoom_laws
            .iter()
            .find(|plan| plan.module == module)
    }

    /// There is deliberately no promotion mutator here.  A close-zoom law can
    /// become renderable only through an authenticated carrier implementation
    /// and a receipt-bound pixel gate, not by toggling graph state at runtime.
    pub fn close_zoom_blockers(&self) -> Vec<(&'static str, &'static str)> {
        self.close_zoom_laws
            .iter()
            .flat_map(|plan| {
                plan.blockers
                    .iter()
                    .copied()
                    .map(move |blocker| (plan.module, blocker))
            })
            .collect()
    }

    pub fn with_water_capture(&self, water_bundle: &NativeD3d11TerrainBundle) -> Result<Self> {
        self.with_dependent_capture(ProductionMapPassKind::Water, water_bundle)
    }

    pub fn with_rivers_capture(&self, rivers_bundle: &NativeD3d11TerrainBundle) -> Result<Self> {
        self.with_dependent_capture(ProductionMapPassKind::Rivers, rivers_bundle)
    }

    pub fn with_borders_capture(&self, borders_bundle: &NativeD3d11TerrainBundle) -> Result<Self> {
        self.with_dependent_capture(ProductionMapPassKind::Borders, borders_bundle)
    }

    pub fn with_trees_capture(&self, trees_bundle: &NativeD3d11TerrainBundle) -> Result<Self> {
        self.with_dependent_capture(ProductionMapPassKind::Trees, trees_bundle)
    }

    pub(crate) fn with_dependent_capture(
        &self,
        kind: ProductionMapPassKind,
        bundle: &NativeD3d11TerrainBundle,
    ) -> Result<Self> {
        if !matches!(
            kind,
            ProductionMapPassKind::Water
                | ProductionMapPassKind::Rivers
                | ProductionMapPassKind::Borders
                | ProductionMapPassKind::Trees
        ) {
            return Err(format!(
                "production map renderer blocked: {} is not a supported validated map-layer bundle yet",
                kind.capture_moment()
            )
            .into());
        }
        if bundle.frame.capture_moment != kind.capture_moment() {
            return Err(format!(
                "production map renderer blocked: {} pass graph cannot run capture moment {}",
                kind.capture_moment(),
                bundle.frame.capture_moment
            )
            .into());
        }
        if ProductionMapPassKind::from_capture_moment(&bundle.frame.capture_moment) != Some(kind) {
            return Err(format!(
                "production map renderer blocked: capture moment {} is not mapped to {:?}",
                bundle.frame.capture_moment, kind
            )
            .into());
        }
        let blockers = native_d3d11_replay_payload_blockers(&bundle.frame, false);
        if !blockers.is_empty() {
            return Err(format!(
                "production map renderer blocked: {} native D3D11 replay payload is incomplete: {}",
                kind.pass_label(),
                blockers.join("; ")
            )
            .into());
        }
        self.validate_dependent_target_bundle(kind, bundle)?;
        let mut graph = self.clone();
        let pass = graph
            .passes
            .iter_mut()
            .find(|pass| pass.kind == kind)
            .ok_or_else(|| {
                format!(
                    "production map renderer blocked: pass graph has no {} pass",
                    kind.capture_moment()
                )
            })?;
        pass.status = ProductionMapPassStatus::Ready;
        pass.blockers.clear();
        Ok(graph)
    }

    pub(crate) fn validate_terrain_bundle(
        &self,
        terrain_bundle: &NativeD3d11TerrainBundle,
    ) -> Result<()> {
        if terrain_bundle.frame.capture_moment != TERRAIN_CAPTURE_MOMENT {
            return Err(format!(
                "production map renderer blocked: terrain pass graph cannot run capture moment {}",
                terrain_bundle.frame.capture_moment
            )
            .into());
        }
        let terrain_pass = self
            .pass(ProductionMapPassKind::Terrain)
            .ok_or("production map renderer blocked: pass graph has no terrain pass")?;
        if terrain_pass.status != ProductionMapPassStatus::Ready {
            return Err(format!(
                "production map renderer blocked: terrain pass is not ready: {}",
                terrain_pass.blockers.join("; ")
            )
            .into());
        }
        let (color, depth) = production_map_target_evidence(&terrain_bundle.frame)?;
        if color.desc.width != self.target.width
            || color.desc.height != self.target.height
            || color.desc.format != self.target.color_format
            || depth.desc.format != self.target.depth_format
            || color.desc.sample_count != self.target.sample_count
            || depth.desc.sample_count != self.target.sample_count
        {
            return Err(
                "production map renderer blocked: terrain bundle target/depth descriptors do not match the app-owned pass graph"
                    .into(),
            );
        }
        self.camera.validate()
    }

    pub(crate) fn validate_dependent_target_bundle(
        &self,
        kind: ProductionMapPassKind,
        bundle: &NativeD3d11TerrainBundle,
    ) -> Result<()> {
        let pass = self.pass(kind).ok_or_else(|| {
            format!(
                "production map renderer blocked: pass graph has no {} pass",
                kind.capture_moment()
            )
        })?;
        let dependency = pass.depends_on.as_slice();
        let [prior_kind] = dependency else {
            return Err(format!(
                "production map renderer blocked: {} pass must have exactly one production dependency, got {:?}",
                kind.capture_moment(),
                pass.depends_on
            )
            .into());
        };
        let prior_pass = self.pass(*prior_kind).ok_or_else(|| {
            format!(
                "production map renderer blocked: {} pass depends on missing {:?} pass",
                kind.capture_moment(),
                prior_kind
            )
        })?;
        if prior_pass.status != ProductionMapPassStatus::Ready {
            return Err(format!(
                "production map renderer blocked: {} pass cannot run before {} is ready: {}",
                kind.pass_label(),
                prior_kind.pass_label(),
                prior_pass.blockers.join("; ")
            )
            .into());
        }
        let (color, depth) = production_map_target_evidence(&bundle.frame)?;
        if color.desc.width != self.target.width
            || color.desc.height != self.target.height
            || color.desc.format != self.target.color_format
            || depth.desc.format != self.target.depth_format
            || color.desc.sample_count != self.target.sample_count
            || depth.desc.sample_count != self.target.sample_count
        {
            return Err(format!(
                "production map renderer blocked: {} bundle target/depth descriptors do not match the app-owned terrain pass graph",
                kind.pass_label()
            )
            .into());
        }
        if kind == ProductionMapPassKind::Water {
            water_refraction_app_owned_terrain_color_srv_desc(&bundle.frame, &color.desc)?;
        }
        for draw in &bundle.frame.draws {
            let state = draw.draw_state.as_ref().ok_or_else(|| {
                format!(
                    "production map renderer blocked: {} draw {} has no captured D3D11 draw state",
                    kind.pass_label(),
                    draw.draw_index
                )
            })?;
            let output_merger = state.output_merger.as_ref().ok_or_else(|| {
                format!(
                    "production map renderer blocked: {} draw {} has no captured output-merger target/depth evidence",
                    kind.pass_label(), draw.draw_index
                )
            })?;
            let draw_color = output_merger.render_targets.first().ok_or_else(|| {
                format!(
                    "production map renderer blocked: {} draw {} has no color render target evidence",
                    kind.pass_label(), draw.draw_index
                )
            })?;
            let draw_depth = output_merger.depth_stencil.as_ref().ok_or_else(|| {
                format!(
                    "production map renderer blocked: {} draw {} has no depth target evidence",
                    kind.pass_label(),
                    draw.draw_index
                )
            })?;
            if draw_color.desc.width != self.target.width
                || draw_color.desc.height != self.target.height
                || draw_color.desc.format != self.target.color_format
                || draw_depth.desc.format != self.target.depth_format
                || draw_color.desc.sample_count != self.target.sample_count
                || draw_depth.desc.sample_count != self.target.sample_count
            {
                return Err(format!(
                    "production map renderer blocked: {} draw {} target/depth descriptors do not match the app-owned terrain pass graph",
                    kind.pass_label(), draw.draw_index
                )
                .into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    use std::fs;

    #[test]
    fn production_map_pass_graph_starts_with_app_owned_terrain_and_blocks_water() {
        let root = temp_test_dir("production_map_pass_graph_terrain_first");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            let mut state = valid_map_frame_draw_state();
            state.depth_stencil = MapFrameDepthStencilState {
                default_state: false,
                stencil_ref: 0,
                desc: Some(valid_depth_enabled_desc()),
            };
            state.output_merger = Some(valid_output_merger_state());
            frame.active_draw.draw_state = Some(state.clone());
            for draw in &mut frame.draws {
                draw.draw_state = Some(state.clone());
            }
            frame
        });
        let bundle =
            NativeD3d11TerrainBundle::load(&path).expect("valid native D3D11 terrain bundle");
        let camera = ProductionMapCamera::new(
            [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            [11.0, 22.0, 33.0],
        )
        .expect("finite production camera");

        let graph = ProductionMapPassGraph::from_terrain_capture(&bundle, camera)
            .expect("terrain capture establishes production pass graph");

        assert_eq!(graph.target.width, 2);
        assert_eq!(graph.target.height, 1);
        assert_eq!(graph.target.color_format, DXGI_FORMAT_B8G8R8A8_UNORM);
        assert_eq!(graph.target.depth_format, 45);
        assert_eq!(graph.target.sample_count, 1);
        assert_eq!(graph.close_zoom_laws.len(), 7);
        assert_eq!(
            graph
                .close_zoom_laws
                .iter()
                .map(|plan| plan.module)
                .collect::<Vec<_>>(),
            vec![
                "shadow_law",
                "building_files",
                "rail_files",
                "occupation_stripe_law",
                "tree_instance_synthesis",
                "camera_interp_law",
                "snow_mud_feed",
            ]
        );
        assert_eq!(
            graph
                .close_zoom_law("occupation_stripe_law")
                .expect("occupation stripe disposition")
                .status,
            ProductionCloseZoomLawStatus::PixelEvidenceBlocked
        );
        assert_eq!(
            graph
                .close_zoom_law("snow_mud_feed")
                .expect("snow/mud disposition")
                .status,
            ProductionCloseZoomLawStatus::PixelEvidenceBlocked
        );
        assert!(graph
            .close_zoom_laws
            .iter()
            .all(|plan| !plan.blockers.is_empty()));
        assert_eq!(
            graph
                .passes
                .iter()
                .map(|pass| pass.kind)
                .collect::<Vec<_>>(),
            vec![
                ProductionMapPassKind::Terrain,
                ProductionMapPassKind::Water,
                ProductionMapPassKind::Rivers,
                ProductionMapPassKind::Borders,
                ProductionMapPassKind::Trees,
                ProductionMapPassKind::Cities,
                ProductionMapPassKind::Overlays,
            ]
        );
        let terrain = graph
            .pass(ProductionMapPassKind::Terrain)
            .expect("terrain pass");
        assert_eq!(terrain.status, ProductionMapPassStatus::Ready);
        assert_eq!(terrain.capture_moment, TERRAIN_CAPTURE_MOMENT);
        assert!(terrain.depends_on.is_empty());
        assert!(terrain.blockers.is_empty());
        let water = graph
            .pass(ProductionMapPassKind::Water)
            .expect("water pass");
        assert_eq!(water.status, ProductionMapPassStatus::Blocked);
        assert_eq!(water.capture_moment, WATER_CAPTURE_MOMENT);
        assert_eq!(water.depends_on, vec![ProductionMapPassKind::Terrain]);
        assert!(water
            .blockers
            .iter()
            .any(|blocker| blocker.contains("app-owned terrain color/depth")));
        for (kind, moment, depends_on, required_text) in [
            (
                ProductionMapPassKind::Rivers,
                RIVERS_CAPTURE_MOMENT,
                ProductionMapPassKind::Water,
                "RiverData",
            ),
            (
                ProductionMapPassKind::Borders,
                BORDERS_CAPTURE_MOMENT,
                ProductionMapPassKind::Rivers,
                "fake province outlines",
            ),
            (
                ProductionMapPassKind::Trees,
                TREES_CAPTURE_MOMENT,
                ProductionMapPassKind::Rivers,
                "map/trees.bmp",
            ),
            (
                ProductionMapPassKind::Cities,
                CITIES_CAPTURE_MOMENT,
                ProductionMapPassKind::Trees,
                "map/cities.bmp",
            ),
            (
                ProductionMapPassKind::Overlays,
                OVERLAYS_CAPTURE_MOMENT,
                ProductionMapPassKind::Cities,
                "static files",
            ),
        ] {
            let pass = graph.pass(kind).expect("remaining production map pass");
            assert_eq!(pass.status, ProductionMapPassStatus::Blocked);
            assert_eq!(pass.capture_moment, moment);
            assert_eq!(pass.depends_on, vec![depends_on]);
            assert!(pass
                .blockers
                .iter()
                .any(|blocker| blocker.contains(required_text)));
        }
        graph
            .validate_terrain_bundle(&bundle)
            .expect("terrain graph accepts matching bundle evidence");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn production_map_pass_graph_promotes_matching_water_capture() {
        let root = temp_test_dir("production_map_pass_graph_water_ready");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let water_path = write_valid_water_map_frame_bundle(&root, |mut frame| {
            add_valid_water_output_merger_state(&root, &mut frame);
            frame
        });
        let terrain_path = root.join(MAP_FRAME_BUNDLE_FILE);
        rewrite_terrain_bundle_with_valid_output_merger_state(&terrain_path);
        let terrain_bundle = NativeD3d11TerrainBundle::load(&terrain_path)
            .expect("valid native D3D11 terrain bundle");
        let water_bundle =
            NativeD3d11TerrainBundle::load_water(&water_path).expect("valid water bundle");
        let camera = terrain_bundle
            .production_camera_from_capture()
            .expect("derive production camera");
        let graph = ProductionMapPassGraph::from_terrain_capture(&terrain_bundle, camera)
            .expect("terrain graph");

        let graph = graph
            .with_water_capture(&water_bundle)
            .expect("matching water capture is part of the production graph");

        let water = graph
            .pass(ProductionMapPassKind::Water)
            .expect("water pass");
        assert_eq!(water.status, ProductionMapPassStatus::Ready);
        assert_eq!(water.depends_on, vec![ProductionMapPassKind::Terrain]);
        assert!(water.blockers.is_empty());
        let rivers = graph
            .pass(ProductionMapPassKind::Rivers)
            .expect("rivers pass");
        assert_eq!(rivers.status, ProductionMapPassStatus::Blocked);
        assert_eq!(rivers.depends_on, vec![ProductionMapPassKind::Water]);
        assert!(rivers
            .blockers
            .iter()
            .any(|blocker| blocker.contains("RiverData")));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn production_map_pass_graph_promotes_validated_map_layers_in_order() {
        let water_root = temp_test_dir("production_map_pass_graph_layer_water");
        let river_root = temp_test_dir("production_map_pass_graph_layer_rivers");
        let border_root = temp_test_dir("production_map_pass_graph_layer_borders");
        let tree_root = temp_test_dir("production_map_pass_graph_layer_trees");
        for root in [&water_root, &river_root, &border_root, &tree_root] {
            fs::create_dir_all(root).expect("create temp bundle dir");
        }
        let water_path = write_valid_water_map_frame_bundle(&water_root, |mut frame| {
            add_valid_water_output_merger_state(&water_root, &mut frame);
            frame
        });
        let terrain_path = water_root.join(MAP_FRAME_BUNDLE_FILE);
        rewrite_terrain_bundle_with_valid_output_merger_state(&terrain_path);
        let rivers_path = write_valid_map_layer_frame_bundle(
            &river_root,
            RIVERS_MAP_FRAME_SPEC,
            &RIVER_FRAME_SHADER_SOURCES,
            &RIVER_FRAME_REQUIRED_SHADER_RESOURCES,
            "river_pixel_shader_5555555555555555.bin",
            "map_vertex_shader_5555555555555555.bin",
            |frame| frame,
        );
        let borders_path = write_valid_map_layer_frame_bundle(
            &border_root,
            BORDERS_MAP_FRAME_SPEC,
            &BORDER_FRAME_SHADER_SOURCES,
            &BORDER_FRAME_REQUIRED_SHADER_RESOURCES,
            "border_pixel_shader_6666666666666666.bin",
            "map_vertex_shader_6666666666666666.bin",
            |frame| frame,
        );
        let trees_path = write_valid_map_layer_frame_bundle(
            &tree_root,
            TREES_MAP_FRAME_SPEC,
            &TREE_FRAME_SHADER_SOURCES,
            &TREE_FRAME_REQUIRED_SHADER_RESOURCES,
            "tree_pixel_shader_7777777777777777.bin",
            "map_vertex_shader_7777777777777777.bin",
            |frame| frame,
        );
        let terrain_bundle = NativeD3d11TerrainBundle::load(&terrain_path)
            .expect("valid native D3D11 terrain bundle");
        let water_bundle =
            NativeD3d11TerrainBundle::load_water(&water_path).expect("valid water bundle");
        let rivers_bundle =
            NativeD3d11TerrainBundle::load_rivers(&rivers_path).expect("valid rivers bundle");
        let borders_bundle =
            NativeD3d11TerrainBundle::load_borders(&borders_path).expect("valid borders bundle");
        let trees_bundle =
            NativeD3d11TerrainBundle::load_trees(&trees_path).expect("valid trees bundle");
        let camera = terrain_bundle
            .production_camera_from_capture()
            .expect("derive production camera");
        let graph = ProductionMapPassGraph::from_terrain_capture(&terrain_bundle, camera)
            .expect("terrain graph");

        let err = graph
            .with_rivers_capture(&rivers_bundle)
            .expect_err("rivers cannot run before water");
        assert!(err.to_string().contains("water is ready"));

        let graph = graph
            .with_water_capture(&water_bundle)
            .expect("water capture promotes");
        let graph = graph
            .with_rivers_capture(&rivers_bundle)
            .expect("rivers capture promotes");
        let graph_without_borders = graph
            .with_trees_capture(&trees_bundle)
            .expect("trees capture promotes after rivers without a separate borders pass");
        assert_eq!(
            graph_without_borders
                .pass(ProductionMapPassKind::Trees)
                .expect("trees pass")
                .status,
            ProductionMapPassStatus::Ready
        );
        let graph = graph
            .with_borders_capture(&borders_bundle)
            .expect("borders capture promotes");
        let graph = graph
            .with_trees_capture(&trees_bundle)
            .expect("trees capture promotes");

        for kind in [
            ProductionMapPassKind::Water,
            ProductionMapPassKind::Rivers,
            ProductionMapPassKind::Borders,
            ProductionMapPassKind::Trees,
        ] {
            let pass = graph.pass(kind).expect("promoted pass");
            assert_eq!(pass.status, ProductionMapPassStatus::Ready);
            assert!(pass.blockers.is_empty());
        }
        assert_eq!(
            graph
                .pass(ProductionMapPassKind::Cities)
                .expect("cities pass")
                .status,
            ProductionMapPassStatus::Blocked
        );

        let _ = fs::remove_dir_all(water_root);
        let _ = fs::remove_dir_all(river_root);
        let _ = fs::remove_dir_all(border_root);
        let _ = fs::remove_dir_all(tree_root);
    }

    #[test]
    fn production_map_pass_graph_rejects_water_native_payload_blockers() {
        let root = temp_test_dir("production_map_pass_graph_water_native_blocker");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let water_path = write_valid_water_map_frame_bundle(&root, |mut frame| {
            add_valid_water_output_merger_state(&root, &mut frame);
            frame
        });
        let terrain_path = root.join(MAP_FRAME_BUNDLE_FILE);
        rewrite_terrain_bundle_with_valid_output_merger_state(&terrain_path);
        let terrain_bundle = NativeD3d11TerrainBundle::load(&terrain_path)
            .expect("valid native D3D11 terrain bundle");
        let mut water_frame: MapFrameRuntimeBundle =
            serde_json::from_slice(&fs::read(&water_path).expect("read water bundle"))
                .expect("parse water bundle");
        for draw in &mut water_frame.draws {
            draw.samplers.clear();
        }
        water_frame.active_draw.samplers.clear();
        let water_bundle = NativeD3d11TerrainBundle {
            map_frame_bundle: water_path.clone(),
            bundle_dir: water_path.parent().expect("bundle dir").to_path_buf(),
            frame: water_frame,
        };
        let camera = terrain_bundle
            .production_camera_from_capture()
            .expect("derive production camera");
        let graph = ProductionMapPassGraph::from_terrain_capture(&terrain_bundle, camera)
            .expect("terrain graph");

        let err = graph
            .with_water_capture(&water_bundle)
            .expect_err("graph must reject incomplete water native replay payloads");

        let message = err.to_string();
        assert!(message.contains("water native D3D11 replay payload"));
        assert!(message.contains("sampler-state"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn production_map_pass_graph_rejects_water_target_mismatch() {
        let root = temp_test_dir("production_map_pass_graph_water_target_mismatch");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let water_path = write_valid_water_map_frame_bundle(&root, |mut frame| {
            add_valid_water_output_merger_state(&root, &mut frame);
            let draw_state = frame.active_draw.draw_state.as_mut().expect("draw state");
            draw_state
                .output_merger
                .as_mut()
                .expect("output merger")
                .render_targets[0]
                .desc
                .width += 1;
            let mismatched_state = frame.active_draw.draw_state.clone();
            for draw in &mut frame.draws {
                draw.draw_state = mismatched_state.clone();
            }
            frame
        });
        let terrain_path = root.join(MAP_FRAME_BUNDLE_FILE);
        rewrite_terrain_bundle_with_valid_output_merger_state(&terrain_path);
        let terrain_bundle = NativeD3d11TerrainBundle::load(&terrain_path)
            .expect("valid native D3D11 terrain bundle");
        let water_bundle =
            NativeD3d11TerrainBundle::load_water(&water_path).expect("valid water bundle");
        let camera = terrain_bundle
            .production_camera_from_capture()
            .expect("derive production camera");
        let graph = ProductionMapPassGraph::from_terrain_capture(&terrain_bundle, camera)
            .expect("terrain graph");

        let err = graph
            .with_water_capture(&water_bundle)
            .expect_err("mismatched water target must fail closed");

        assert!(err.to_string().contains("target/depth descriptors"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn production_map_pass_graph_rejects_water_second_draw_target_mismatch() {
        let root = temp_test_dir("production_map_pass_graph_water_second_draw_target_mismatch");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let water_path = write_valid_water_map_frame_bundle(&root, |mut frame| {
            add_valid_water_output_merger_state(&root, &mut frame);
            let active_draw_index = frame.active_draw.draw_index;
            let other_draw = frame
                .draws
                .iter_mut()
                .find(|draw| draw.draw_index != active_draw_index)
                .expect("second water draw");
            other_draw
                .draw_state
                .as_mut()
                .expect("draw state")
                .output_merger
                .as_mut()
                .expect("output merger")
                .render_targets[0]
                .desc
                .format += 1;
            frame
        });
        let terrain_path = root.join(MAP_FRAME_BUNDLE_FILE);
        rewrite_terrain_bundle_with_valid_output_merger_state(&terrain_path);
        let terrain_bundle = NativeD3d11TerrainBundle::load(&terrain_path)
            .expect("valid native D3D11 terrain bundle");
        let water_bundle =
            NativeD3d11TerrainBundle::load_water(&water_path).expect("valid water bundle");
        let camera = terrain_bundle
            .production_camera_from_capture()
            .expect("derive production camera");
        let graph = ProductionMapPassGraph::from_terrain_capture(&terrain_bundle, camera)
            .expect("terrain graph");

        let err = graph
            .with_water_capture(&water_bundle)
            .expect_err("mismatched second water draw target must fail closed");

        assert!(err.to_string().contains("water draw"));
        assert!(err.to_string().contains("target/depth descriptors"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn production_map_pass_graph_rejects_water_refraction_target_mismatch() {
        let root = temp_test_dir("production_map_pass_graph_water_refraction_mismatch");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let water_path = write_valid_water_map_frame_bundle(&root, |mut frame| {
            for draw in &mut frame.draws {
                let resource = draw
                    .shader_resources
                    .iter_mut()
                    .find(|resource| {
                        resource.declared_name.as_deref() == Some(WATER_REFRACTION_RESOURCE_SYMBOL)
                    })
                    .expect("WaterRefraction resource");
                resource
                    .view_desc
                    .as_mut()
                    .expect("WaterRefraction SRV desc")
                    .format = DXGI_FORMAT_R8G8B8A8_UNORM;
            }
            frame.active_draw = frame.draws[0].clone();
            frame
        });
        let terrain_path = root.join(MAP_FRAME_BUNDLE_FILE);
        rewrite_terrain_bundle_with_valid_output_merger_state(&terrain_path);
        let terrain_bundle = NativeD3d11TerrainBundle::load(&terrain_path)
            .expect("valid native D3D11 terrain bundle");
        let water_bundle =
            NativeD3d11TerrainBundle::load_water(&water_path).expect("valid water bundle");
        let camera = terrain_bundle
            .production_camera_from_capture()
            .expect("derive production camera");
        let graph = ProductionMapPassGraph::from_terrain_capture(&terrain_bundle, camera)
            .expect("terrain graph");

        let err = graph
            .with_water_capture(&water_bundle)
            .expect_err("water must not sample a mismatched refraction target");

        let message = err.to_string();
        assert!(message.contains(WATER_REFRACTION_RESOURCE_SYMBOL));
        assert!(message.contains("SRV format"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn production_map_pass_graph_rejects_water_refraction_draw_descriptor_drift() {
        let root = temp_test_dir("production_map_pass_graph_water_refraction_drift");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let water_path = write_valid_water_map_frame_bundle(&root, |mut frame| {
            let active_draw_index = frame.active_draw.draw_index;
            let other_draw = frame
                .draws
                .iter_mut()
                .find(|draw| draw.draw_index != active_draw_index)
                .expect("second water draw");
            other_draw
                .shader_resources
                .iter_mut()
                .find(|resource| {
                    resource.declared_name.as_deref() == Some(WATER_REFRACTION_RESOURCE_SYMBOL)
                })
                .expect("WaterRefraction resource")
                .view_desc
                .as_mut()
                .expect("WaterRefraction SRV desc")
                .mip_levels = 2;
            frame
        });
        let terrain_path = root.join(MAP_FRAME_BUNDLE_FILE);
        rewrite_terrain_bundle_with_valid_output_merger_state(&terrain_path);
        let terrain_bundle = NativeD3d11TerrainBundle::load(&terrain_path)
            .expect("valid native D3D11 terrain bundle");
        let water_bundle =
            NativeD3d11TerrainBundle::load_water(&water_path).expect("valid water bundle");
        let camera = terrain_bundle
            .production_camera_from_capture()
            .expect("derive production camera");
        let graph = ProductionMapPassGraph::from_terrain_capture(&terrain_bundle, camera)
            .expect("terrain graph");

        let err = graph
            .with_water_capture(&water_bundle)
            .expect_err("water draws must agree on app-owned refraction SRV shape");

        assert!(err.to_string().contains(WATER_REFRACTION_RESOURCE_SYMBOL));
        assert!(err.to_string().contains("single Texture2D mip0"));
        let _ = fs::remove_dir_all(&root);
    }
}
