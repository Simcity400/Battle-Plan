use viewer_gpu::gpu_contract::Targets;
use std::{collections::BTreeSet, io};

use eframe::wgpu;
use content_reader_map::map::{MapData, ProvinceKind};
use replay_data::rows::tables;

use super::{
    border_pass::{BorderAssets, land_class, run_indices},
    borders::{BorderClass, BorderMeshes, Vertex},
};
use map_display_data::landscape::Regions;
use viewer_gpu::{gpu_contract::{DEPTH_FORMAT, buffer, invalid}, textures::{ColorSpace, upload}};
use viewer_foundation::tables::rows;

pub const TRANSPARENCY: f32 = 1.0;

const VERTEX_STRIDE: u64 = 16;
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Zone {
    states: BTreeSet<u32>,
    flag: Vec<bool>,
    land: Vec<bool>,
}

impl Zone {
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

    pub fn splits(&self, a: u32, b: u32) -> bool {
        self.land(a) && self.land(b) && self.demilitarized(a) != self.demilitarized(b)
    }

    fn demilitarized(&self, province: u32) -> bool {
        self.flag.get(province as usize).copied().unwrap_or(false)
    }

    fn land(&self, province: u32) -> bool {
        self.land.get(province as usize).copied().unwrap_or(false)
    }

    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }

    pub fn states(&self) -> &BTreeSet<u32> {
        &self.states
    }
}

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

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ZonePlan {
    runs: Vec<PlannedRun>,
    edges: usize,
}

#[derive(Clone, Debug, PartialEq)]
struct PlannedRun {
    class: BorderClass,
    vertices: Vec<u8>,
    indices: Vec<u32>,
    fine: u32,
}

#[derive(Default)]
struct Gathered {
    fine: (Vec<Vertex>, Vec<u32>),
    coarse: (Vec<Vertex>, Vec<u32>),
}

impl ZonePlan {
    pub fn new(meshes: &BorderMeshes, regions: &Regions, zone: &Zone) -> Self {
        if zone.is_empty() {
            return Self::default();
        }
        Self::matching(meshes, regions, |a, b| zone.splits(a, b))
    }

    pub fn matching(
        meshes: &BorderMeshes,
        regions: &Regions,
        highlight: impl Fn(u32, u32) -> bool,
    ) -> Self {
        Self::select(meshes, regions, highlight, false)
    }

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
        let mut per_class: Vec<Gathered> = BorderClass::ALL.iter().map(|_| Gathered::default()).collect();
        for (mesh, hidden, draw) in draws {
            if !highlight(draw.province_a, draw.province_b) {
                continue;
            }
            let strip = |chunks: &'a [Vec<Vertex>], base: u32, count: u32| {
                chunks.get(draw.chunk as usize)?.get(base as usize..(base + count) as usize)
            };
            let (Some(fine), Some(coarse)) = (
                strip(&mesh.chunks, draw.base_vertex, draw.vertex_count),
                strip(&mesh.coarse_chunks, draw.coarse_base_vertex, draw.coarse_vertex_count),
            ) else {
                continue;
            };
            let class = match (mesh.class, hidden) {
                (_, true) => BorderClass::Province,
                (BorderClass::Province, false) => land(draw.province_a, draw.province_b),
                (other, false) => other,
            };
            let gathered = &mut per_class[class.bucket()];
            for ((vertices, indices), strip) in [(&mut gathered.fine, fine), (&mut gathered.coarse, coarse)] {
                run_indices(vertices.len() as u32, strip.len() as u32 / 2, indices);
                vertices.extend_from_slice(strip);
            }
            plan.edges += 1;
        }
        for (class, gathered) in BorderClass::ALL.iter().zip(per_class) {
            let Gathered { fine: (mut vertices, mut indices), coarse } = gathered;
            if indices.is_empty() {
                continue;
            }
            let fine = indices.len() as u32;
            let base = vertices.len() as u32;
            vertices.extend(coarse.0);
            indices.extend(coarse.1.into_iter().map(|index| index + base));
            plan.runs.push(PlannedRun {
                class: *class,
                vertices: vertex_bytes(&vertices),
                indices,
                fine,
            });
        }
        plan
    }
}

#[cfg(test)]
impl ZonePlan {
    pub(crate) fn triangles(&self, coarse: bool) -> Vec<[[f32; 3]; 3]> {
        let mut triangles = Vec::new();
        for run in &self.runs {
            let position = |index: u32| -> [f32; 3] {
                let at = index as usize * VERTEX_STRIDE as usize;
                std::array::from_fn(|axis| {
                    f32::from_le_bytes(run.vertices[at + axis * 4..][..4].try_into().expect("four bytes"))
                })
            };
            let drawn = viewer_scene_base::line_lod::drawn(run.fine, run.indices.len() as u32 - run.fine, coarse);
            let indices = &run.indices[drawn.start as usize..drawn.end as usize];
            triangles.extend(indices.chunks(3).map(|triangle| std::array::from_fn(|corner| position(triangle[corner]))));
        }
        triangles
    }
}

struct Run {
    class: BorderClass,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    fine: u32,
    coarse: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ZoneEdges {
    Highlighted,
    Ordinary {
        camera_height: f32,
        fades: [[f32; 2]; 2],
        province_borders: bool,
    },
    Hidden,
}

impl ZoneEdges {
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

pub struct DemilitarizedPass {
    compiled: Result<super::border_shader::Border, String>,
    textures: Vec<wgpu::TextureView>,
    runs: Vec<Run>,
    edges: usize,
    color: [f32; 4],
    hover: Vec<Run>,
    hover_color: Option<[f32; 4]>,
    selection: Vec<Run>,
    selection_color: Option<[f32; 4]>,
}

impl DemilitarizedPass {
    pub fn texel_reads(&self) -> Option<&str> {
        self.compiled.as_ref().ok()?.texel_reads.as_deref()
    }

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
            selection: Vec::new(),
            selection_color: None,
        })
    }

    pub fn set_zone(
        &mut self,
        device: &wgpu::Device,
        meshes: &BorderMeshes,
        regions: &Regions,
        zone: &Zone,
    ) -> io::Result<()> {
        self.set_plan(device, &ZonePlan::new(meshes, regions, zone))
    }

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

    pub fn set_selection(&mut self, device: &wgpu::Device, plan: Option<(&ZonePlan, [f32; 4])>) -> io::Result<()> {
        match plan {
            Some((plan, color)) => {
                Self::upload_plan(device, plan, &mut self.selection)?;
                self.selection_color = Some(color);
            }
            None => {
                self.selection.clear();
                self.selection_color = None;
            }
        }
        Ok(())
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
                fine: run.fine,
                coarse: run.indices.len() as u32 - run.fine,
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
        coarse: bool,
    ) -> io::Result<()> {
        let mut batches = Vec::new();
        for class in BorderClass::ALL {
            if let Some(color) = self.selection_color {
                batches.push(self.batch(class, &self.selection, color, coarse));
            }
            let mut zone = self.batch(class, &self.runs, self.color, coarse);
            edges.apply(class, &mut zone.alpha, &mut zone.selection, &mut zone.runs);
            batches.push(zone);
            if let Some(color) = self.hover_color {
                batches.push(self.batch(class, &self.hover, color, coarse));
            }
        }
        viewer_gpu::gpu_contract::compiled(&self.compiled)?
            .draw(arena, encoder, targets, inputs, &batches, true)
            .map(|_| ())
    }

    fn batch<'a>(
        &'a self,
        class: BorderClass,
        runs: &'a [Run],
        color: [f32; 4],
        coarse: bool,
    ) -> super::border_shader::Batch<'a> {
        let runs = runs
            .iter()
            .filter(|run| run.class == class)
            .map(|run| super::border_shader::Run {
                vertices: &run.vertices,
                indices: &run.indices,
                range: viewer_scene_base::line_lod::drawn(run.fine, run.coarse, coarse),
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
    use content_reader_map::map::{CountryColors, ProvinceDefinition};

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
        use super::super::border_pass::Topology;
        use viewer_gpu::province_map::{HeightField, ProvinceMap};
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
        assert!(!zone.splits(1, 2));
        assert!(zone.splits(2, 3));
        assert!(zone.splits(3, 1));
        assert!(!zone.splits(3, 3));
    }

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
                flag(4242, 1),
            ]));
        assert_eq!(
            recorded_states(&frame).unwrap(),
            BTreeSet::from([42, 51, 978])
        );
    }

    #[test]
    fn an_incomplete_table_draws_no_zone_at_all() {
        use replay_data::rows::TableRows;
        use replay_frame::frame::TableView;
        let whole_states =
            || TableView::Complete(TableRows::States(vec![state(42), state(51), state(978)]));
        let mut frame = frame();
        *frame.tables[replay_data::rows::TableId::States.index()] = whole_states();
        assert!(recorded_states(&frame).unwrap().is_empty());
        *frame.tables[replay_data::rows::TableId::StateFlags.index()] = TableView::Truncated {
            rows: TableRows::StateFlags(vec![flag(42, 1)]),
            missing: 1_080,
        };
        assert!(recorded_states(&frame).unwrap().is_empty());
        *frame.tables[replay_data::rows::TableId::StateFlags.index()] =
            TableView::Complete(TableRows::StateFlags(vec![flag(42, 1)]));
        *frame.tables[replay_data::rows::TableId::States.index()] = TableView::Truncated {
            rows: TableRows::States(vec![state(42)]),
            missing: 1_080,
        };
        assert!(recorded_states(&frame).unwrap().is_empty());
        *frame.tables[replay_data::rows::TableId::States.index()] = whole_states();
        assert_eq!(recorded_states(&frame).unwrap(), BTreeSet::from([42]));
    }
}
