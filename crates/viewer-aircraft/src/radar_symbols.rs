use shader_pipeline::gpu::stats::CountedPasses as _;
use super::{arrow_shader::Arrow, order_pass::NativeInputs};
use eframe::wgpu;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, io, sync::Arc};
use viewer_gpu::{
    gpu_contract::{DEPTH_FORMAT, SCENE_FORMAT, Targets, buffer},
    province_map::HeightField,
    ribbon_content::SymbolPage,
    textures::{ColorSpace, upload},
};
use viewer_foundation::visibility::Families;

const FIXED5: f64 = 100_000.0;

const GAME_FRAME: f32 = 1.0 / 30.0;

fn turn(angles: &mut [f64], rotation: i64, flip: bool, elapsed: f32) {
    if rotation == 0 || !elapsed.is_finite() || elapsed <= 0.0 {
        return;
    }
    let whole = FIXED5 * f64::from(std::f32::consts::TAU);
    let step = rotation as f64 * f64::from(elapsed) / f64::from(GAME_FRAME);
    for (slot, angle) in angles.iter_mut().enumerate() {
        let step = if flip && slot % 2 == 1 { -step } else { step };
        *angle = (*angle + step).rem_euclid(whole);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Symbol {
    pub country: u32,
    pub state: u32,
    pub own: bool,
    pub roster: bool,
    pub position: [f32; 3],
    pub diameter: f32,
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
            out.groups(
                groups.iter().map(|group| {
                    let country = links.get(&group.country).copied();
                    Group {
                        country,
                        ordinal: group.ordinal,
                        admitted: match (country, &families) {
                            (Some(country), _) if country == ours => Ok(true),
                            (Some(country), Ok(families)) => Ok(families.family(country, ours)),
                            (_, Err(reason)) => Err(reason.to_string()),
                            (None, Ok(_)) => Ok(false),
                        },
                        state: group.state,
                        radius: group.radius,
                    }
                }),
                ours,
                heights,
                centers,
            );
            Ok(())
        };
        if let Err(reason) = read() {
            *out.refused.entry(reason).or_default() += 1;
        }
        out
    }

    pub fn groups(&mut self, groups: impl IntoIterator<Item = Group>, ours: u32, heights: HeightField<'_>, centers: &Centers) {
        let out = self;
        {
            let mut ordered = Vec::new();
            for group in groups {
                let Some(country) = group.country else {
                    *out.refused
                        .entry("RadarCoverage country is unresolved".into())
                        .or_default() += 1;
                    continue;
                };
                let admitted = match &group.admitted {
                    Ok(admitted) => *admitted,
                    Err(reason) => {
                        *out.refused
                            .entry(format!("allied radar admission: {reason}"))
                            .or_default() += 1;
                        false
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
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub country: Option<u32>,
    pub ordinal: u32,
    pub admitted: Result<bool, String>,
    pub state: Option<u32>,
    pub radius: Option<i64>,
}

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
    angles: Vec<f64>,
    field: Field,
    random: viewer_models::entity_animation::Random,
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
            random: viewer_models::entity_animation::Random::from_counter(seed),
            shown: false,
            hovered: None,
        })
    }

    pub fn set(&mut self, field: &Field) {
        if &self.field != field {
            self.field = field.clone();
        }
    }

    pub fn advance(&mut self, elapsed: f32) {
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
            self.angles.push(f64::from(angle) * FIXED5);
        }
        turn(&mut self.angles, self.step, self.label == "Air ranges", elapsed);
    }

    #[expect(clippy::too_many_arguments, reason = "the arena, encoder and targets, plus the native inputs, planes, time and visibility of one draw")]
    pub fn draw(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
        native: NativeInputs<'_>,
        planes: &viewer_gpu::province_admission::Planes,
        time: f32,
        visible: bool,
    ) -> io::Result<()> {
        if !visible {
            return Ok(());
        }
        use content_reader_gui::pdxshader::program::Stage;
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
            if !viewer_gpu::province_admission::admits_box(
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
                    (symbol
                        .angle
                        .map_or_else(|| self.angles.get(slot).copied().unwrap_or(0.), |angle| angle as f64)
                        / FIXED5) as f32,
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
            if self.program.reads_time || (symbol.angle.is_none() && self.step != 0) {
                arena.request_frame();
            }
            draws.push(bound);
        }
        if draws.is_empty() {
            return Ok(());
        }
        let mut pass = encoder.counted_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("radar coverage map symbols"),
            color_attachments: &[Some(targets.scene_attachment(wgpu::LoadOp::Load))],
            depth_stencil_attachment: Some(targets.depth_attachment(wgpu::LoadOp::Load)),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        self.program.begin(&mut pass);
        shader_pipeline::gpu::stats::state();
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        let mut previous = None;
        for bound in &draws {
            bound.bind_after(&mut pass, previous);
            previous = Some(bound);
            shader_pipeline::gpu::stats::draw();
            pass.draw(0..4, 0..1);
        }
        Ok(())
    }

    pub fn texel_reads(&self) -> Option<&str> {
        self.program.texel_reads.as_deref()
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
    fn a_sweep_turns_by_elapsed_time_whatever_the_display_s_rate() {
        let rotation = 700;
        let over_a_second = |rate: u32| {
            let mut angles = vec![0.0_f64, 0.0];
            for _ in 0..rate {
                turn(&mut angles, rotation, true, 1.0 / rate as f32);
            }
            angles
        };
        let sixty = over_a_second(60);
        for rate in [120, 240] {
            for (slot, (at_sixty, at_rate)) in sixty.iter().zip(&over_a_second(rate)).enumerate() {
                assert!(
                    (at_sixty - at_rate).abs() < 1.0,
                    "slot {slot} stands at {at_rate} after a second at {rate} Hz, {at_sixty} at 60 Hz"
                );
            }
        }
        let whole = FIXED5 * f64::from(std::f32::consts::TAU);
        assert!((sixty[0] - 30.0 * rotation as f64).abs() < 1.0, "{}", sixty[0]);
        assert!((sixty[1] - (whole - 30.0 * rotation as f64)).abs() < 1.0, "{}", sixty[1]);
        let mut standing = vec![5.0_f64];
        turn(&mut standing, rotation, false, 0.0);
        turn(&mut standing, 0, false, 1.0);
        assert_eq!(standing, vec![5.0]);
    }

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
