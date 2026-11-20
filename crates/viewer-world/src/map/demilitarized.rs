//! Demilitarised-zone borders, using selection reason 7.
//!
//! A highlighted edge uses the ordinary traced geometry and base-material
//! texture in the secondary bucket bank. Exactly one province must have a
//! nonzero recorded demilitarised-state byte. Missing states read as zero;
//! [`Zone::splits`] requires both sides to be land, excluding coasts. Edges
//! inside a zone receive no highlight. Each edge enters one bucket, so the
//! highlight replaces its ordinary border draw.
//!
//! The palette is resolved from content. Entry 0 is clear, entries 1 through 9
//! are the named highlight colours, and entries 10 onward are custom colours.
//! Entry 7 is `BORDER_COLOR_DEMILITARIZED`. Width, texture and pulse shading come
//! from the ordinary border material and installed compiled shader.
//!
//! Pulse time follows the presentation clock, independent of simulation time.
//! Windows animate locally; headless dumps use the default clock. Highlighted
//! rows retain full transparency at every camera height. Ordinary province and
//! state fades apply only to rows with selection reason zero.
//!
//! Each wrap view draws ordinary province/state/country/impassable buckets,
//! then straits and gradient borders, then sea/sea-region and highlighted
//! buckets. The demilitarised pass follows the ordinary border pass.
//!
//! [`recorded_states`] joins complete StateFlags and States tables, preserving
//! the raw byte before selecting nonzero state ids. Missing or partial inputs
//! withhold the layer with a diagnostic. The dump-only `--demilitarized` option
//! adds explicit state ids to this recorded set.
use viewer_foundation::map::gpu_contract::Targets;
use std::{collections::BTreeSet, io};

use eframe::wgpu;
use installed_content_core::{map::MapData, map::ProvinceKind};
use replay_data::rows::tables;

use {
    super::{
        border_pass::{BorderAssets, land_class},
        borders::{BorderClass, BorderMeshes, Vertex},
        political::Regions,
    },
    viewer_foundation::map::{
        gpu_contract::{DEPTH_FORMAT, buffer, invalid},
        tables::rows,
        textures::{ColorSpace, upload},
    },
};

/// `vTime_Transparency.y` for a secondary-bank draw; see the module docs for why it is 1.
pub const TRANSPARENCY: f32 = 1.0;

/// Bytes one border vertex occupies: `float3 position` then `6G16_SINT uv`.
const VERTEX_STRIDE: u64 = 16;
/// One hour's demilitarised states, projected onto provinces.
///
/// [`Zone::splits`] is the engine's reason-7 predicate: both provinces are land and exactly one of
/// them sits in a demilitarised state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Zone {
    /// The state ids, kept so two zones compare cheaply.
    states: BTreeSet<u32>,
    /// Per province id: the province's state is demilitarised.
    flag: Vec<bool>,
    /// Per province id: whether the resolved province definition marks it as land.
    land: Vec<bool>,
}

impl Zone {
    /// Projects `states` through this hour's province-to-state mapping.
    ///
    /// `map` supplies the land bit: `map/definition.csv`'s province kind, the same source
    /// `super::border_pass::Topology` reads it from.
    pub fn new(map: &MapData, regions: &Regions, states: &BTreeSet<u32>) -> Self {
        let mut flag = vec![false; regions.state_of.len()];
        let mut land = vec![false; regions.state_of.len()];
        if !states.is_empty() {
            for (province, state) in regions.state_of.iter().enumerate() {
                flag[province] = states.contains(state);
            }
            for (&id, definition) in &map.definitions {
                if let Some(slot) = land.get_mut(id as usize) {
                    *slot = !matches!(definition.kind, ProvinceKind::Sea | ProvinceKind::Lake);
                }
            }
        }
        Self {
            states: states.clone(),
            flag,
            land,
        }
    }

    /// Whether the boundary between two provinces is a demilitarised-zone edge.
    pub fn splits(&self, a: u32, b: u32) -> bool {
        self.land(a) && self.land(b) && self.demilitarized(a) != self.demilitarized(b)
    }

    fn demilitarized(&self, province: u32) -> bool {
        self.flag.get(province as usize).copied().unwrap_or(false)
    }

    fn land(&self, province: u32) -> bool {
        self.land.get(province as usize).copied().unwrap_or(false)
    }

    /// Whether the hour knows of no demilitarised state at all, in which case nothing is drawn and
    /// the border pass has nothing to leave out.
    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }

    /// The state ids behind the zone, in ascending order.
    pub fn states(&self) -> &BTreeSet<u32> {
        &self.states
    }
}

/// The demilitarised state ids this hour records.
///
/// StateFlags supplies one raw demilitarised byte per state alongside States. Any nonzero byte
/// qualifies without normalization to one. Complete tables include zero flags, so their nonzero ids
/// define the zone.
///
/// The join is `StateFlags.state` to `States.id`: the flags table is keyed by a state id and the
/// hour's own `States` table is what says which state ids exist. A flag for a state this hour does
/// not carry is not projected onto the map.
///
/// **An incomplete table yields nothing at all.** A truncated walk reached real rows, but the rows
/// it did not reach are rows the hour is *silent* about, and the engine's reason-7 predicate is an
/// exclusive or: reading silence as "not demilitarised" would not draw a shorter outline, it would
/// draw a **wrong** one, cutting a zone in half along a boundary the game draws no line on. So
/// either table being anything but complete leaves the zone empty and the hour states the gap
/// through `viewer_foundation::map::tables::limitations`.
pub fn recorded_states(frame: &replay_frame::frame::DecodedFrame) -> io::Result<BTreeSet<u32>> {
    let flags = rows::<tables::StateFlags>(frame)?;
    let states = rows::<tables::States>(frame)?;
    if !flags.is_complete() || !states.is_complete() {
        return Ok(BTreeSet::new());
    }
    let existing: BTreeSet<u32> = states.rows().iter().map(|state| state.id).collect();
    Ok(flags
        .rows()
        .iter()
        .filter(|flag| flag.demilitarized != 0)
        .map(|flag| flag.state)
        .filter(|state| existing.contains(state))
        .collect())
}

/// One hour's highlighted strips, selected without a device so a builder
/// thread can do it ahead of the frame that shows the hour.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ZonePlan {
    runs: Vec<PlannedRun>,
    edges: usize,
}

#[derive(Clone, Debug, PartialEq)]
struct PlannedRun {
    class: BorderClass,
    /// In `borders::Vertex`'s 16-byte layout.
    vertices: Vec<u8>,
    indices: Vec<u32>,
}

impl ZonePlan {
    /// Selects the edges `zone` splits, grouped by base material.
    ///
    /// The strips are copied out of the traced meshes rather than re-traced: a highlighted chain is
    /// the same boundary the border pass already holds, so the only per-hour work is selecting the
    /// edges the zone splits and grouping them by base material.
    pub fn new(meshes: &BorderMeshes, regions: &Regions, zone: &Zone) -> Self {
        if zone.is_empty() {
            return Self::default();
        }
        Self::matching(meshes, regions, |a, b| zone.splits(a, b))
    }

    /// Selects ordinary-material strips for the demilitarised secondary bank.
    pub fn matching(
        meshes: &BorderMeshes,
        regions: &Regions,
        highlight: impl Fn(u32, u32) -> bool,
    ) -> Self {
        Self::select(meshes, regions, highlight, false)
    }

    /// Custom reasons can also claim native material 0xff, remapped to Province.
    pub fn custom_highlight(
        meshes: &BorderMeshes,
        regions: &Regions,
        highlight: impl Fn(u32, u32) -> bool,
    ) -> Self {
        Self::select(meshes, regions, highlight, true)
    }

    fn select(
        meshes: &BorderMeshes,
        regions: &Regions,
        highlight: impl Fn(u32, u32) -> bool,
        include_hidden: bool,
    ) -> Self {
        let draws = BorderClass::ALL
            .iter()
            .map(|&class| (meshes.mesh(class), false))
            .chain(include_hidden.then_some((&meshes.hidden, true)))
            .flat_map(|(mesh, hidden)| mesh.draws.iter().map(move |draw| (mesh, hidden, draw)));
        Self::gather(draws, highlight, |a, b| land_class(regions, a, b))
    }

    /// The plan [`Self::new`] makes, from `draws` alone (each a class and the draw's index in that
    /// class's mesh): equal to it when `draws` holds every draw `splits` keeps, such as the draws of
    /// the zone's provinces. `land` is the land class of a province mesh draw's two provinces.
    pub fn of_draws(
        meshes: &BorderMeshes,
        mut draws: Vec<(BorderClass, u32)>,
        splits: impl Fn(u32, u32) -> bool,
        land: impl Fn(u32, u32) -> BorderClass,
    ) -> Self {
        draws.sort_unstable_by_key(|&(class, at)| (class.bucket(), at));
        draws.dedup();
        let draws = draws.into_iter().filter_map(|(class, at)| {
            let mesh = meshes.mesh(class);
            mesh.draws.get(at as usize).map(|draw| (mesh, false, draw))
        });
        Self::gather(draws, splits, land)
    }

    fn gather<'a>(
        draws: impl Iterator<Item = (&'a super::borders::BorderMesh, bool, &'a super::borders::BorderDraw)>,
        highlight: impl Fn(u32, u32) -> bool,
        land: impl Fn(u32, u32) -> BorderClass,
    ) -> Self {
        let mut plan = Self::default();
        // One bucket per base material; a land edge's material is the hour's own class, every other
        // edge keeps the class the content settled at trace time.
        let mut per_class: Vec<(Vec<Vertex>, Vec<u32>)> = BorderClass::ALL
            .iter()
            .map(|_| (Vec::new(), Vec::new()))
            .collect();
        // Material 0xff is remapped to Province only when a custom highlight
        // claims it; retaining these strips closes coasts and isolated islands.
        for (mesh, hidden, draw) in draws {
            if !highlight(draw.province_a, draw.province_b) {
                continue;
            }
            let Some(chunk) = mesh.chunks.get(draw.chunk as usize) else {
                continue;
            };
            let first = draw.base_vertex as usize;
            let last = first + draw.vertex_count as usize;
            let Some(strip) = chunk.get(first..last) else {
                continue;
            };
            let class = match (mesh.class, hidden) {
                (_, true) => BorderClass::Province,
                (BorderClass::Province, false) => land(draw.province_a, draw.province_b),
                (other, false) => other,
            };
            let (vertices, indices) = &mut per_class[class.bucket()];
            let base = vertices.len() as u32;
            vertices.extend_from_slice(strip);
            for segment in 0..(draw.vertex_count / 2).saturating_sub(1) {
                let quad = base + segment * 2;
                indices.extend_from_slice(&[
                    quad,
                    quad + 2,
                    quad + 1,
                    quad + 2,
                    quad + 3,
                    quad + 1,
                ]);
            }
            plan.edges += 1;
        }
        for (class, (vertices, indices)) in BorderClass::ALL.iter().zip(per_class) {
            if indices.is_empty() {
                continue;
            }
            plan.runs.push(PlannedRun {
                class: *class,
                vertices: vertex_bytes(&vertices),
                indices,
            });
        }
        plan
    }
}

/// One class's highlighted geometry.
struct Run {
    class: BorderClass,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
}

/// How the edges a demilitarised zone splits from the ordinary border buckets draw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ZoneEdges {
    /// In the zone's highlight colour, as the game draws them.
    Highlighted,
    /// As the ordinary land borders they would be without the zone, while the zone display is
    /// hidden and land borders show: the ordinary pass leaves these edges to this one.
    Ordinary {
        camera_height: f32,
        fades: [[f32; 2]; 2],
        /// Whether the map mode draws province borders ([`super::borders::class_alpha`]).
        province_borders: bool,
    },
    /// Not at all, while land borders are hidden.
    Hidden,
}

impl ZoneEdges {
    /// A class's zone batch as these edges draw it: the ordinary pass's palette entry (none) and
    /// fade, or no runs. Every texture stays bound, so later passes keep the same inputs.
    fn apply<T>(
        self,
        class: BorderClass,
        alpha: &mut f32,
        selection: &mut [f32; 4],
        runs: &mut Vec<T>,
    ) {
        match self {
            Self::Highlighted => {}
            Self::Ordinary {
                camera_height,
                fades,
                province_borders,
            } => {
                *selection = [0.0; 4];
                *alpha = super::borders::class_alpha(class, camera_height, fades, province_borders);
                if *alpha <= 0.0 {
                    runs.clear();
                }
            }
            Self::Hidden => runs.clear(),
        }
    }
}

/// The demilitarised-zone border pass: the engine's secondary bucket bank, reason 7.
pub struct DemilitarizedPass {
    compiled: Result<super::border_shader::Border, String>,
    textures: Vec<wgpu::TextureView>,
    runs: Vec<Run>,
    edges: usize,
    /// `NGraphics.BORDER_COLOR_DEMILITARIZED_*`, palette entry 7.
    color: [f32; 4],
    /// Hover highlights share the pipeline/textures, with their own palette entry.
    hover: Vec<Run>,
    hover_color: Option<[f32; 4]>,
}

impl DemilitarizedPass {
    /// How the program drawn reads the shadow screen buffer by texel ([`super::shadow::texel_reads`]).
    pub fn texel_reads(&self) -> Option<&str> {
        self.compiled.as_ref().ok()?.texel_reads.as_deref()
    }

    /// Builds the pipeline and uploads the six class textures.
    ///
    /// The textures are the same six `map/terrain/border_<class>_<quality>.dds` the ordinary border pass
    /// binds, because a highlighted chain keeps its base material.
    pub fn new(
        device: &wgpu::Device,
        samples: u32,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        assets: &BorderAssets,
    ) -> io::Result<Self> {
        let textures = BorderClass::ALL
            .iter()
            .zip(&assets.classes)
            .map(|(class, texture)| {
                let label = format!("demilitarized border_{}", class.texture_name());
                Ok(upload(device, queue, &label, texture, ColorSpace::Srgb)?.view)
            })
            .collect::<io::Result<Vec<_>>>()?;
        Ok(Self {
            compiled: super::border_shader::Border::new(
                device,
                &assets.border_shader,
                samples,
                format,
                DEPTH_FORMAT,
            )
            .map_err(|error| error.to_string()),
            textures,
            runs: Vec::new(),
            edges: 0,
            color: assets.demilitarized_color,
            hover: Vec::new(),
            hover_color: None,
        })
    }

    /// Rebuilds the highlighted geometry for one hour's zone.
    ///
    /// The strips are copied out of the traced meshes rather than re-traced: a highlighted chain is
    /// the same boundary the border pass already holds, so the only per-hour work is selecting the
    /// edges the zone splits and grouping them by base material.
    pub fn set_zone(
        &mut self,
        device: &wgpu::Device,
        meshes: &BorderMeshes,
        regions: &Regions,
        zone: &Zone,
    ) -> io::Result<()> {
        self.set_plan(device, &ZonePlan::new(meshes, regions, zone))
    }

    /// Uploads the strips [`ZonePlan::new`] selected.
    pub fn set_plan(&mut self, device: &wgpu::Device, plan: &ZonePlan) -> io::Result<()> {
        self.edges = plan.edges;
        Self::upload_plan(device, plan, &mut self.runs)
    }

    pub fn set_hover(
        &mut self,
        device: &wgpu::Device,
        plan: &ZonePlan,
        color: [f32; 4],
    ) -> io::Result<()> {
        Self::upload_plan(device, plan, &mut self.hover)?;
        self.hover_color = Some(color);
        Ok(())
    }

    pub fn clear_hover(&mut self) {
        self.hover.clear();
        self.hover_color = None;
    }

    fn upload_plan(device: &wgpu::Device, plan: &ZonePlan, runs: &mut Vec<Run>) -> io::Result<()> {
        runs.clear();
        for run in &plan.runs {
            runs.push(Run {
                class: run.class,
                vertices: buffer(
                    device,
                    &format!("highlighted {} strips", run.class.texture_name()),
                    wgpu::BufferUsages::VERTEX,
                    &run.vertices,
                )?,
                count: run.indices.len() as u32,
                indices: buffer(
                    device,
                    "highlighted indices",
                    wgpu::BufferUsages::INDEX,
                    bytemuck::cast_slice(&run.indices),
                )?,
            });
        }
        Ok(())
    }

    /// Province boundaries this hour's zone highlights.
    pub fn edge_count(&self) -> usize {
        self.edges
    }

    pub fn draw(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
        inputs: super::border_shader::Inputs<'_>,
        edges: ZoneEdges,
    ) -> io::Result<()> {
        // Native buckets follow material order, with reason-byte order inside
        // each: DMZ 7 before the custom palette entries at 10+. The faction hover outline is
        // interaction feedback, not a border, and draws whatever the border switches say.
        let mut batches = Vec::new();
        for class in BorderClass::ALL {
            let mut zone = self.batch(class, &self.runs, self.color);
            edges.apply(class, &mut zone.alpha, &mut zone.selection, &mut zone.runs);
            batches.push(zone);
            if let Some(color) = self.hover_color {
                batches.push(self.batch(class, &self.hover, color));
            }
        }
        // One initialization and every material's texture bind are required
        // even when its runs are empty. Keep the complete bank.
        viewer_foundation::map::gpu_contract::compiled(&self.compiled)?
            .draw(arena, encoder, targets, inputs, &batches, true)
            .map(|_| ())
    }

    fn batch<'a>(
        &'a self,
        class: BorderClass,
        runs: &'a [Run],
        color: [f32; 4],
    ) -> super::border_shader::Batch<'a> {
        let runs = runs
            .iter()
            .filter(|run| run.class == class)
            .map(|run| super::border_shader::Run {
                vertices: &run.vertices,
                indices: &run.indices,
                count: run.count,
            })
            .collect::<Vec<_>>();
        super::border_shader::Batch {
            texture: &self.textures[class.bucket()],
            alpha: TRANSPARENCY,
            selection: color,
            runs,
        }
    }
}

/// The vertex bytes of one strip list, in `borders::Vertex`'s 16-byte layout.
fn vertex_bytes(vertices: &[Vertex]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vertices.len() * VERTEX_STRIDE as usize);
    for vertex in vertices {
        for value in vertex.position {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for value in vertex.uv {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    bytes
}

/// Parses the `--demilitarized <id>,<id>,...` verification override.
pub fn parse_states(value: &str) -> io::Result<BTreeSet<u32>> {
    let mut states = BTreeSet::new();
    for field in value.split(',') {
        let field = field.trim();
        if field.is_empty() {
            continue;
        }
        let id = field.parse::<u32>().map_err(|_| {
            invalid(format!(
                "`{field}` is not a state id; --demilitarized takes a comma-separated list"
            ))
        })?;
        states.insert(id);
    }
    if states.is_empty() {
        return Err(invalid("--demilitarized needs at least one state id"));
    }
    Ok(states)
}

#[cfg(test)]
mod tests {
    use super::*;
    use installed_content_core::{map::CountryColors, map::ProvinceDefinition};

    #[test]
    fn hidden_zones_draw_their_edges_as_ordinary_borders_or_not_at_all() {
        let zone = |edges: ZoneEdges, class| {
            let (mut alpha, mut selection, mut runs) =
                (TRANSPARENCY, [0.5, 0.0, 0.0, 1.0], vec![(); 2]);
            edges.apply(class, &mut alpha, &mut selection, &mut runs);
            (alpha, selection, runs.len())
        };
        assert_eq!(
            zone(ZoneEdges::Highlighted, BorderClass::Country),
            (TRANSPARENCY, [0.5, 0.0, 0.0, 1.0], 2)
        );
        // Shown as the ordinary pass draws the class: no palette entry and its fade.
        let fades = [[10.0, 20.0], [30.0, 40.0]];
        let near = ZoneEdges::Ordinary {
            camera_height: 5.0,
            fades,
            province_borders: true,
        };
        assert_eq!(zone(near, BorderClass::Province), (1.0, [0.0; 4], 2));
        let far = ZoneEdges::Ordinary {
            camera_height: 25.0,
            fades,
            province_borders: true,
        };
        assert_eq!(
            zone(far, BorderClass::Province).2,
            0,
            "faded out like the ordinary pass"
        );
        assert_eq!(
            zone(far, BorderClass::State).0,
            super::super::borders::class_alpha(BorderClass::State, 25.0, fades, true)
        );
        assert_eq!(zone(ZoneEdges::Hidden, BorderClass::Country).2, 0);
        // A map mode without province borders gives the zone's province edges no alpha either.
        let resources = ZoneEdges::Ordinary {
            camera_height: 5.0,
            fades,
            province_borders: false,
        };
        assert_eq!(zone(resources, BorderClass::Province), (0.0, [0.0; 4], 0));
        assert_eq!(zone(resources, BorderClass::State).0, 1.0);
    }

    #[test]
    fn theatre_outline_closes_coasts_and_island_without_drawing_internal_or_double_edges() {
        use {
            super::super::border_pass::Topology,
            viewer_foundation::map::province_map::{HeightField, ProvinceMap},
        };
        // Native province adjacency for this raster is 1-2, 1-3, 2-3,
        // 1-4 (wrap), 2-4, 3-4 and 4-5. Province 5 is an isolated island.
        let mut map = map();
        map.width = 8;
        map.height = 4;
        map.province_ids = vec![
            1, 1, 2, 2, 4, 4, 4, 4, 1, 1, 2, 2, 4, 5, 5, 4, 3, 3, 3, 3, 4, 5, 5, 4, 3, 3, 3, 3, 4,
            4, 4, 4,
        ];
        map.definitions.insert(
            5,
            ProvinceDefinition {
                id: 5,
                rgb: [5, 0, 0],
                kind: ProvinceKind::Land,
                continent: 0,
            },
        );
        let regions = regions(&[0, 42, 42, 7, 0, 42]);
        let topology = Topology::new(&map, &Default::default(), &Default::default(), &[]).unwrap();
        let values = vec![95; 32];
        let meshes = super::super::borders::build(
            &ProvinceMap {
                width: 8,
                height: 4,
                ids: &map.province_ids,
            },
            &|a, b| topology.trace_class(a, b),
            &|id| topology.is_water(id),
            &HeightField {
                width: 8,
                height: 4,
                values: &values,
            },
            1.5,
        )
        .unwrap();
        let pairs = |mesh: &super::super::borders::BorderMesh| {
            mesh.draws
                .iter()
                .map(|draw| (draw.province_a, draw.province_b))
                .collect::<BTreeSet<_>>()
        };
        assert_eq!(
            pairs(meshes.mesh(BorderClass::Province)),
            BTreeSet::from([(1, 2), (1, 3), (2, 3)])
        );
        assert_eq!(
            pairs(&meshes.hidden),
            BTreeSet::from([(1, 4), (2, 4), (3, 4), (4, 5)])
        );
        // The highlighted province set {1,2,5} has five
        // perimeter runs. The 1-2 interior and the wholly external 3-4 stay out.
        let provinces = BTreeSet::from([1, 2, 5]);
        let highlight = |a, b| provinces.contains(&a) != provinces.contains(&b);
        let plan = ZonePlan::custom_highlight(&meshes, &regions, highlight);
        assert_eq!(plan.edges, 5);
        assert_eq!(ZonePlan::matching(&meshes, &regions, highlight).edges, 2);
        let classes: BTreeSet<_> = plan.runs.iter().map(|run| run.class).collect();
        assert_eq!(
            classes,
            BTreeSet::from([BorderClass::Province, BorderClass::State])
        );
        let zone = Zone::new(&map, &regions, &BTreeSet::from([42]));
        assert_eq!(ZonePlan::new(&meshes, &regions, &zone).edges, 2);
        // Custom reason overrides DMZ reason 7; these strips enter one bank.
        assert_eq!(
            ZonePlan::matching(&meshes, &regions, |a, b| zone.splits(a, b)
                && !highlight(a, b))
            .edges,
            0
        );
    }

    fn regions(state_of: &[u32]) -> Regions {
        Regions {
            country_of: vec![0; state_of.len()].into(),
            owner_of: vec![0; state_of.len()].into(),
            state_of: state_of.to_vec().into(),
            country_colors: Default::default(),
            warnings: Vec::new(),
            strategic_attributes: Default::default(),
            mode: None,
        }
    }

    /// Provinces 1, 2 and 3 are land and province 4 is a sea province.
    fn map() -> MapData {
        MapData {
            width: 4,
            height: 1,
            province_ids: vec![1, 2, 3, 4],
            definitions: [1, 2, 3, 4]
                .into_iter()
                .map(|id| {
                    (
                        id,
                        ProvinceDefinition {
                            id,
                            rgb: [id as u8, 0, 0],
                            kind: match id {
                                4 => ProvinceKind::Sea,
                                _ => ProvinceKind::Land,
                            },
                            continent: 0,
                        },
                    )
                })
                .collect(),
            country_colors: CountryColors::default(),
        }
    }

    /// Provinces 1 and 2 are in demilitarised state 42, province 3 in state 7 and province 4 is
    /// the stateless sea province.
    fn zone() -> Zone {
        Zone::new(
            &map(),
            &regions(&[0, 42, 42, 7, 0]),
            &BTreeSet::from([42, 51, 978]),
        )
    }

    #[test]
    fn only_the_outline_of_the_zone_splits() {
        let zone = zone();
        // Inside the zone: both sides demilitarised.
        assert!(!zone.splits(1, 2));
        // The outline against another state.
        assert!(zone.splits(2, 3));
        assert!(zone.splits(3, 1));
        // Outside it entirely.
        assert!(!zone.splits(3, 3));
    }

    /// Coasts are excluded even when a missing water-side state supplies zero.
    #[test]
    fn a_coast_takes_the_land_bit_exemption() {
        let zone = zone();
        assert!(!zone.splits(1, 4));
        assert!(!zone.splits(3, 4));
    }

    #[test]
    fn the_override_parses_a_state_list() {
        assert_eq!(
            parse_states("42,51,978").unwrap(),
            BTreeSet::from([42, 51, 978])
        );
        assert_eq!(parse_states(" 42 , 42 ").unwrap(), BTreeSet::from([42]));
        assert!(parse_states("42,x").is_err());
        assert!(parse_states("").is_err());
    }

    fn frame() -> replay_frame::frame::DecodedFrame {
        replay_frame::frame::DecodedFrame {
            branch: 1,
            hour: 60_832_633,
            tables: std::array::from_fn(|_| replay_frame::frame::TableView::Unavailable {
                reason: "the collector had not grown its buffer yet".into(),
            })
            .map(Into::into),
            metrics: replay_frame::frame::Metrics::default(),
        }
    }

    fn flag(state: u32, demilitarized: u8) -> replay_data::rows::StateFlagRow {
        replay_data::rows::StateFlagRow {
            state,
            demilitarized,
            ..Default::default()
        }
    }

    fn state(id: u32) -> replay_data::rows::StateRow {
        replay_data::rows::StateRow {
            id,
            ..Default::default()
        }
    }

    /// The engine's own test is `!= 0` on an unnormalised byte, and a complete
    /// table carries the false flags too, so the zone is the non-zero rows.
    #[test]
    fn the_zone_is_every_state_whose_recorded_byte_is_not_zero() {
        use replay_data::rows::TableRows;
        use replay_frame::frame::TableView;
        let mut frame = frame();
        *frame.tables[replay_data::rows::TableId::States.index()] =
            TableView::Complete(TableRows::States(vec![
                state(42),
                state(51),
                state(64),
                state(978),
            ]));
        *frame.tables[replay_data::rows::TableId::StateFlags.index()] =
            TableView::Complete(TableRows::StateFlags(vec![
                flag(42, 1),
                flag(51, 1),
                flag(64, 0),
                flag(978, 7),
                // A flag for a state this hour's States table does not carry is
                // not projected onto the map.
                flag(4242, 1),
            ]));
        assert_eq!(
            recorded_states(&frame).unwrap(),
            BTreeSet::from([42, 51, 978])
        );
    }

    /// Silence is not "not demilitarised": the reason-7 predicate is an
    /// exclusive or, so a half-read table would draw a line the game does not.
    #[test]
    fn an_incomplete_table_draws_no_zone_at_all() {
        use replay_data::rows::TableRows;
        use replay_frame::frame::TableView;
        let whole_states =
            || TableView::Complete(TableRows::States(vec![state(42), state(51), state(978)]));
        // Unavailable: no rows and no count.
        let mut frame = frame();
        *frame.tables[replay_data::rows::TableId::States.index()] = whole_states();
        assert!(recorded_states(&frame).unwrap().is_empty());
        // Truncated: real rows reached, but the rest are unknown.
        *frame.tables[replay_data::rows::TableId::StateFlags.index()] = TableView::Truncated {
            rows: TableRows::StateFlags(vec![flag(42, 1)]),
            missing: 1_080,
        };
        assert!(recorded_states(&frame).unwrap().is_empty());
        // Whole flags, but no state ids to join them to.
        *frame.tables[replay_data::rows::TableId::StateFlags.index()] =
            TableView::Complete(TableRows::StateFlags(vec![flag(42, 1)]));
        *frame.tables[replay_data::rows::TableId::States.index()] = TableView::Truncated {
            rows: TableRows::States(vec![state(42)]),
            missing: 1_080,
        };
        assert!(recorded_states(&frame).unwrap().is_empty());
        // Both whole: the zone is drawn.
        *frame.tables[replay_data::rows::TableId::States.index()] = whole_states();
        assert_eq!(recorded_states(&frame).unwrap(), BTreeSet::from([42]));
    }
}
