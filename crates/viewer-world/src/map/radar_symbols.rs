//! Radar range symbols in the shared map-arrow manager. Station icons and
//! this display switch are independent; an icon hover keeps one range visible.
use super::{arrow_shader::Arrow, order_pass::NativeInputs};
use eframe::wgpu;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, io, sync::Arc};
use viewer_foundation::map::{
    gpu_contract::{DEPTH_FORMAT, SCENE_FORMAT, Targets, buffer},
    province_map::HeightField,
    ribbon_content::SymbolPage,
    textures::{ColorSpace, upload},
    visibility::Families,
};

#[derive(Clone, Debug, PartialEq)]
pub struct Symbol {
    pub country: u32,
    pub state: u32,
    pub own: bool,
    /// Own/family groups belong to the enabled overlay; other groups can
    /// still be supplied by a station hover while the overlay is off.
    pub roster: bool,
    pub position: [f32; 3],
    pub diameter: f32,
    /// A simulation-derived orientation replaces pooled presentation rotation.
    pub angle: Option<i64>,
}

impl Symbol {
    fn shown(&self, enabled: bool, hovered: Option<(u32, u32)>) -> bool {
        if enabled {
            self.roster
        } else {
            hovered == Some((self.state, self.country))
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Field {
    pub symbols: Vec<Symbol>,
    pub refused: BTreeMap<String, usize>,
}

/// Host-template centres are content facts, computed once rather than on camera rebuilds.
pub struct Centers(BTreeMap<u32, Result<[i32; 2], String>>);

impl Centers {
    pub fn get(&self, state: u32) -> Result<[i32; 2], String> {
        self.0
            .get(&state)
            .cloned()
            .unwrap_or_else(|| Err(format!("host state {state} is not in recorded content")))
    }
    pub fn new(
        states: &BTreeMap<u32, Vec<u32>>,
        bounds: &BTreeMap<u32, [f32; 4]>,
        width: i32,
    ) -> Self {
        Self(
            states
                .iter()
                .map(|(&state, provinces)| (state, state_center(provinces, bounds, width)))
                .collect(),
        )
    }
}

impl Field {
    /// The enabled roster uses direct faction/subject relationships. All
    /// recorded groups remain available for the independent station hover.
    pub fn read(
        frame: &DecodedFrame,
        ours: u32,
        heights: HeightField<'_>,
        centers: &Centers,
    ) -> Self {
        let mut out = Self::default();
        let mut read = || -> Result<(), String> {
            let links: BTreeMap<_, _> = frame
                .complete::<tables::CountryLinks>()
                .map_err(|e| e.to_string())?
                .iter()
                .map(|r| (r.id, r.canonical_slot))
                .collect();
            let families = Families::read(frame, &links);
            let groups = frame
                .complete::<tables::RadarCoverage>()
                .map_err(|e| e.to_string())?;
            let mut ordered = Vec::new();
            for group in groups {
                let Some(&country) = links.get(&group.country) else {
                    *out.refused
                        .entry("RadarCoverage country is unresolved".into())
                        .or_default() += 1;
                    continue;
                };
                let admitted = if country == ours {
                    true
                } else {
                    match &families {
                        Ok(families) => families.family(country, ours),
                        Err(reason) => {
                            *out.refused
                                .entry(format!("allied radar admission: {reason}"))
                                .or_default() += 1;
                            false
                        }
                    }
                };
                ordered.push((country != ours, country, group.ordinal, admitted, group));
            }
            ordered.sort_by_key(|&(foreign, country, ordinal, _, _)| (foreign, country, ordinal));
            for (foreign, country, _, roster, group) in ordered {
                let (Some(state), Some(radius)) = (group.state, group.radius) else {
                    *out.refused
                        .entry("radar range lacks certified radius or host state".into())
                        .or_default() += 1;
                    continue;
                };
                let centre = centers.0.get(&state).cloned().unwrap_or_else(|| {
                    Err(format!(
                        "radar host state {state} is not in recorded content"
                    ))
                });
                let [x, z] = match centre {
                    Ok(centre) => centre,
                    Err(reason) => {
                        *out.refused.entry(reason).or_default() += 1;
                        continue;
                    }
                };
                out.symbols.push(Symbol {
                    state,
                    country,
                    own: !foreign,
                    roster,
                    position: [
                        x as f32,
                        heights.native_height(x as f32, z as f32),
                        z as f32,
                    ],
                    diameter: radius.wrapping_mul(2) as f32 / 100_000.,
                    angle: None,
                });
            }
            Ok(())
        };
        if let Err(reason) = read() {
            *out.refused.entry(reason).or_default() += 1;
        }
        out
    }
}

/// The host template combines ordinary and seam-shifted province boxes,
/// keeps the narrower one (ordinary on ties), then halves integer extents.
fn state_center(
    provinces: &[u32],
    bounds: &BTreeMap<u32, [f32; 4]>,
    width: i32,
) -> Result<[i32; 2], String> {
    let mut ordinary: Option<[i32; 4]> = None;
    let mut shifted = None;
    let union = |acc: &mut Option<[i32; 4]>, b: [i32; 4]| {
        *acc = Some(match *acc {
            None => b,
            Some(a) => [
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[2].max(b[2]),
                a[3].max(b[3]),
            ],
        });
    };
    for &province in provinces {
        let b = bounds
            .get(&province)
            .ok_or_else(|| format!("radar host province {province} has no proved pixel bounds"))?
            .map(|v| v as i32);
        union(&mut ordinary, b);
        let delta = if b[0] < width / 2 { width } else { 0 };
        union(
            &mut shifted,
            [
                b[0].wrapping_add(delta),
                b[1],
                b[2].wrapping_add(delta),
                b[3],
            ],
        );
    }
    let Some(mut b) = ordinary else {
        return Ok([0, 0]);
    };
    let s = shifted.unwrap();
    if s[2].wrapping_sub(s[0]) < b[2].wrapping_sub(b[0]) {
        b = s;
    }
    let mut x = b[0].wrapping_add(b[2].wrapping_sub(b[0]) / 2);
    if x < 0 {
        x = x.wrapping_add(width);
    } else if x >= width {
        x = x.wrapping_sub(width);
    }
    Ok([x, b[1].wrapping_add(b[3].wrapping_sub(b[1]) / 2)])
}

/// Native symbol quad: XYZ followed by UV3, four strip vertices.
const QUAD: [[f32; 6]; 4] = [
    [-0.5, 0., -0.5, 0., 1., 0.],
    [0.5, 0., -0.5, 1., 1., 0.],
    [-0.5, 0., 0.5, 0., 0., 0.],
    [0.5, 0., 0.5, 1., 0., 0.],
];

pub struct Pass {
    label: &'static str,
    program: Arrow,
    texture: wgpu::TextureView,
    vertices: wgpu::Buffer,
    size: f32,
    colors: [[f32; 4]; 3],
    step: i64,
    angles: Vec<i64>,
    field: Field,
    random: super::entity_animation::Random,
    pub shown: bool,
    pub hovered: Option<(u32, u32)>,
}

impl Pass {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        samples: u32,
        page: &SymbolPage,
        effect: &Arc<shader_pipeline::library::Effect>,
    ) -> io::Result<Self> {
        Self::named(device, queue, samples, page, effect, "Radar symbols")
    }

    pub fn named(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        samples: u32,
        page: &SymbolPage,
        effect: &Arc<shader_pipeline::library::Effect>,
        label: &'static str,
    ) -> io::Result<Self> {
        let texture = upload(
            device,
            queue,
            "radar range symbol",
            &page.texture,
            ColorSpace::Srgb,
        )?;
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos();
        Ok(Self {
            label,
            program: Arrow::symbol(
                device,
                effect,
                &page.technique,
                samples,
                SCENE_FORMAT,
                DEPTH_FORMAT,
            )?,
            texture: texture.view,
            vertices: buffer(
                device,
                "map symbol quad",
                wgpu::BufferUsages::VERTEX,
                bytemuck::cast_slice(&QUAD),
            )?,
            size: page.size,
            colors: page.colors,
            step: page.rotation,
            angles: Vec::new(),
            field: Field::default(),
            random: super::entity_animation::Random::from_counter(seed),
            shown: false,
            hovered: None,
        })
    }

    pub fn set(&mut self, field: &Field) {
        if &self.field != field {
            self.field = field.clone();
        }
    }

    /// Pooled wrappers retain angles by draw slot, including when their group changes.
    pub fn advance(&mut self) {
        let count = self
            .field
            .symbols
            .iter()
            .filter(|s| s.shown(self.shown, self.hovered))
            .count();
        self.angles.truncate(count);
        while self.angles.len() < count {
            let angle = if self.step == 0 {
                0.
            } else {
                (self.random.next_word() as f32 / 2_147_483_648.) * std::f32::consts::TAU
            };
            self.angles
                .push(((if angle >= 0. { 1. } else { 0. }) + angle * 100_000. - 0.5) as i64);
        }
        if self.step != 0 {
            for (slot, angle) in self.angles.iter_mut().enumerate() {
                let step = if self.label == "Air ranges" && slot % 2 == 1 {
                    self.step.wrapping_neg()
                } else {
                    self.step
                };
                let value = (angle.wrapping_add(step) as f32 / 100_000.) % std::f32::consts::TAU;
                *angle = ((if value >= 0. { 1. } else { 0. }) + value * 100_000. - 0.5) as i64;
            }
        }
    }

    // Keep native frame inputs and the existing render targets borrowed for this draw.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
        native: NativeInputs<'_>,
        planes: &viewer_foundation::map::province_admission::Planes,
        time: f32,
        visible: bool,
    ) -> io::Result<()> {
        if !visible {
            return Ok(());
        }
        use installed_content_core::pdxshader::program::Stage;
        use shader_pipeline::binding;
        let mut textures: BTreeMap<_, _> = native
            .textures
            .iter()
            .enumerate()
            .map(|(i, v)| (binding(i as u32 + 2, Stage::Pixel), (*v).clone()))
            .collect();
        textures.insert(binding(1, Stage::Pixel), self.texture.clone());
        let mut draws = Vec::new();
        for (slot, symbol) in self
            .field
            .symbols
            .iter()
            .filter(|s| s.shown(self.shown, self.hovered))
            .enumerate()
        {
            let [x, y, z] = symbol.position;
            let half = symbol.diameter * self.size * 0.5;
            if !viewer_foundation::map::province_admission::admits_box(
                [[x - half, y, z - half], [x + half, y + 0.01, z + half]],
                0.,
                planes,
            ) {
                continue;
            }
            let color = if !self.shown {
                0
            } else if self.hovered == Some((symbol.state, symbol.country)) {
                2
            } else {
                usize::from(!symbol.own)
            };
            let constants = [
                self.colors[color],
                [
                    symbol.position[0],
                    symbol.position[1],
                    symbol.position[2],
                    symbol.diameter * self.size,
                ],
                [
                    time,
                    0.,
                    0.,
                    symbol
                        .angle
                        .unwrap_or_else(|| self.angles.get(slot).copied().unwrap_or(0))
                        as f32
                        / 100_000.,
                ],
            ];
            let constants = bytemuck::cast_slice(&constants);
            let bound =
                self.program
                    .bindings(arena, native.scene, native.grid, constants, &textures)?;
            if let Some(retained) = native.retained {
                let mut retained = retained.borrow_mut();
                self.program.retain(
                    &mut retained,
                    native.scene,
                    native.grid,
                    constants,
                    &textures,
                );
                retained.bind_vertex(0, 24, bytemuck::cast_slice(&QUAD).into());
            }
            draws.push(bound);
        }
        if draws.is_empty() {
            return Ok(());
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("radar coverage map symbols"),
            color_attachments: &[Some(targets.scene_attachment(wgpu::LoadOp::Load))],
            depth_stencil_attachment: Some(targets.depth_attachment(wgpu::LoadOp::Load)),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        self.program.begin(&mut pass);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        for bound in draws {
            bound.bind(&mut pass);
            pass.draw(0..4, 0..1);
        }
        Ok(())
    }

    pub fn warnings(&self) -> Vec<String> {
        let mut lines: Vec<_> = self
            .field
            .refused
            .iter()
            .map(|(reason, count)| format!("{}: {count} refused groups: {reason}", self.label))
            .collect();
        if self
            .field
            .symbols
            .iter()
            .any(|symbol| symbol.angle.is_none())
        {
            lines.push(format!("{} rotation starts a fresh presentation: the game's unsaved initial angles are not recorded.", self.label));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_centres_choose_the_narrower_wrapped_integer_box_and_keep_ordinary_ties() {
        let bounds = BTreeMap::from([
            (1, [0., 0., 2., 2.]),
            (2, [98., 4., 100., 8.]),
            (3, [50., 0., 52., 2.]),
        ]);
        assert_eq!(state_center(&[], &bounds, 100), Ok([0, 0]));
        assert_eq!(state_center(&[1, 2], &bounds, 100), Ok([0, 4]));
        assert_eq!(state_center(&[1, 3], &bounds, 100), Ok([26, 1]));
        assert!(
            state_center(&[4], &bounds, 100)
                .unwrap_err()
                .contains("province 4")
        );
    }
    #[test]
    fn own_family_roster_and_foreign_hover_use_certified_ranges_without_attache_admission() {
        use replay_data::{
            rows::CountryLinkRow, rows::FactionMemberRow, rows::RadarCoverageRow, rows::TableId,
            rows::TableRows,
        };
        use replay_frame::{frame::Metrics, frame::TableView};
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 1,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "synthetic".into(),
                }
                .into()
            }),
        };
        frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
            (1..=3)
                .map(|slot| CountryLinkRow {
                    id: slot * 100,
                    canonical_slot: slot,
                })
                .collect(),
        ))
        .into();
        frame.tables[TableId::FactionMembers.index()] =
            TableView::Complete(TableRows::FactionMembers(vec![
                FactionMemberRow {
                    country: 100,
                    faction: 4,
                    ordinal: 0,
                },
                FactionMemberRow {
                    country: 200,
                    faction: 4,
                    ordinal: 1,
                },
            ]))
            .into();
        frame.tables[TableId::Subjects.index()] =
            TableView::Complete(TableRows::Subjects(vec![])).into();
        frame.tables[TableId::RadarCoverage.index()] =
            TableView::Complete(TableRows::RadarCoverage(
                [300, 200, 100, 100]
                    .into_iter()
                    .enumerate()
                    .map(|(i, country)| RadarCoverageRow {
                        country,
                        ordinal: i as u32,
                        state: Some(i as u32 + 1),
                        radius: (i != 3).then_some(150_000),
                        provinces: vec![],
                    })
                    .collect(),
            ))
            .into();
        let heights = HeightField {
            width: 2,
            height: 2,
            values: &[200; 4],
        };
        let states = (1..=4).map(|id| (id, vec![1])).collect();
        let bounds = BTreeMap::from([(1, [0., 0., 2., 2.])]);
        let centers = Centers::new(&states, &bounds, 2);
        let field = Field::read(&frame, 1, heights, &centers);
        assert_eq!(
            field
                .symbols
                .iter()
                .map(|s| (s.state, s.own, s.roster, s.diameter, s.position))
                .collect::<Vec<_>>(),
            [
                (3, true, true, 3., [1., 20., 1.]),
                (2, false, true, 3., [1., 20., 1.]),
                (1, false, false, 3., [1., 20., 1.]),
            ]
        );
        assert_eq!(
            field.symbols.iter().filter(|s| s.shown(true, None)).count(),
            2
        );
        let foreign = field.symbols.iter().find(|s| s.country == 3).unwrap();
        assert!(!foreign.shown(true, Some((1, 3))));
        assert!(foreign.shown(false, Some((1, 3))));
        assert!(!foreign.shown(false, None));
        assert_eq!(field.refused.values().sum::<usize>(), 1);
        // A gap in relationships does not take away a proved own range.
        frame.tables[TableId::Subjects.index()] = TableView::Unavailable {
            reason: "gap".into(),
        }
        .into();
        let field = Field::read(&frame, 1, heights, &centers);
        assert_eq!(
            field.symbols.iter().filter(|s| s.shown(true, None)).count(),
            1
        );
        assert!(field.symbols.iter().any(|s| s.shown(false, Some((1, 3)))));
    }
}
