//! World-space country names drawn into the HDR scene before post-processing.
//!
//! [`super::names`] places names in map-pixel space. This module groups
//! provinces, resolves display strings, adds world-space terrain heights and
//! submits the recorded content's compiled name shader. Font atlases are UNORM;
//! country names receive scene grading. Screen-space labels draw afterward.
//!
//! Connected components group provinces by controller. Raster neighbours wrap
//! horizontally, stop at the poles and include the diagonal of a four-province
//! crossing. Every authored adjacency row also joins its pair, regardless of
//! crossing kind. Province zero contributes no raster neighbours.
//!
//! Political mode draws only country names, faded by the content cutoff.
//! Heights are bilinearly sampled from the bottom-up raster and clamped per
//! corner to water height. Removing the diagonal saddle component makes each
//! quad planar while preserving its mean and both slopes; mixed heights are
//! not clamped again.
use std::cell::RefCell;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
};
use viewer_foundation::map::gpu_contract::DEPTH_FORMAT;
use viewer_foundation::map::name_draw::{Placements, WorldName, next_generation};

use super::map_mode::GroupLabels;
use eframe::wgpu;
use installed_content_core::{ResolvedLayers, adjacencies, map::MapData, map::ProvinceKind};
use installed_content_country_names::country_names::CountryNames;
use installed_content_image::IndexedImage;
use installed_content_localisation::{localisation, localisation::Localisation};
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;

use {
    super::{
        name_shader::Vertex as NameVertex,
        names::{self, NameInput, PlacedName, ProvinceBounds},
    },
    viewer_foundation::map::{
        camera::{Camera, MapSize},
        gpu_contract::Targets,
        province_admission::{admits_box, side_planes},
        province_map::{ProvinceMap, WATER_HEIGHT, world_from_pixel},
        sprites::IconTextures,
        tables::core,
        text,
    },
};

/// World units per heightmap byte.
const HEIGHT_SCALE: f32 = 0.1;

/// The province adjacency graph the component grouping walks.
#[derive(Clone, Debug, Default)]
pub struct ProvinceGraph {
    ids: Vec<u32>,
    rows: Vec<usize>,
    flat: Vec<usize>,
    edges: usize,
}

fn mark(bits: &mut [u64], id: usize) {
    bits[id / 64] |= 1u64 << (id % 64);
}

fn marked(bits: &[u64], id: usize) -> bool {
    bits.get(id / 64)
        .is_some_and(|word| *word & (1u64 << (id % 64)) != 0)
}

impl ProvinceGraph {
    /// Builds the graph from a province image and the authored adjacency rows.
    ///
    /// Pairs are collected into a set first, so the cost is one pass over the
    /// image and the memory is the number of distinct adjacent pairs, not the
    /// number of boundary pixels.
    pub fn build(
        width: u32,
        height: u32,
        ids: &[u32],
        rows: &[adjacencies::Adjacency],
    ) -> io::Result<Self> {
        let (w, h) = (width as usize, height as usize);
        if w == 0 || h == 0 || ids.len() != w * h {
            return Err(invalid(format!(
                "province image is {width}x{height} but carries {} ids",
                ids.len()
            )));
        }
        let mut pairs: HashSet<(u32, u32)> = HashSet::new();
        let mut add = |a: u32, b: u32| {
            if a != b {
                pairs.insert((a.min(b), a.max(b)));
            }
        };
        let at = |x: usize, y: usize| ids[y * w + x];
        for y in (0..h).rev() {
            for x in 0..w {
                let source = at(x, y);
                if source == 0 {
                    continue;
                }
                let right_x = if x + 1 == w { 0 } else { x + 1 };
                let left_x = if x == 0 { w - 1 } else { x - 1 };
                let right = at(right_x, y);
                let next = if y > 0 { at(x, y - 1) } else { 0 };
                let left = at(left_x, y);
                let previous = if y + 1 < h { at(x, y + 1) } else { 0 };
                for target in [right, next, left, previous] {
                    if target != 0 {
                        add(source, target);
                    }
                }
                let diagonal = if y > 0 { at(right_x, y - 1) } else { 0 };
                let four = [source, right, next, diagonal];
                if four.iter().all(|&id| id != 0)
                    && (0..4).all(|a| (a + 1..4).all(|b| four[a] != four[b]))
                {
                    add(source, diagonal);
                }
            }
        }
        for row in rows {
            add(row.from, row.to);
        }
        let edges = pairs.len();
        let mut neighbours: HashMap<u32, Vec<u32>> = HashMap::new();
        for (a, b) in pairs {
            neighbours.entry(a).or_default().push(b);
            neighbours.entry(b).or_default().push(a);
        }
        for list in neighbours.values_mut() {
            list.sort_unstable();
        }
        let mut ids = engine_native::allocation::collect_vec(neighbours.keys().copied())?;
        ids.sort_unstable();
        let mut rows = engine_native::allocation::vec(ids.len() + 1)?;
        let mut flat = engine_native::allocation::vec(neighbours.values().map(Vec::len).sum())?;
        rows.push(0);
        for id in &ids {
            flat.extend(neighbours[id].iter().map(|next| {
                ids.binary_search(next)
                    .expect("every endpoint has an ordinal")
            }));
            rows.push(flat.len());
        }
        Ok(Self {
            ids,
            rows,
            flat,
            edges,
        })
    }

    /// Splits `members` into connected components, each sorted ascending.
    ///
    /// The components come back in ascending order of their smallest member, so
    /// a headless render always lays the same areas out in the same order.
    pub fn components(&self, members: &BTreeSet<u32>) -> Vec<Vec<u32>> {
        if members.is_empty() {
            return Vec::new();
        }
        let words = self.ids.len().div_ceil(64);
        let mut member = vec![0u64; words];
        for id in members {
            if let Ok(ordinal) = self.ids.binary_search(id) {
                mark(&mut member, ordinal);
            }
        }
        let mut seen = vec![0u64; words];
        let mut out = Vec::new();
        for &start in members {
            let Ok(ordinal) = self.ids.binary_search(&start) else {
                out.push(vec![start]);
                continue;
            };
            if marked(&seen, ordinal) {
                continue;
            }
            mark(&mut seen, ordinal);
            let mut component = vec![start];
            let mut stack = vec![ordinal];
            while let Some(province) = stack.pop() {
                for &next in self.row(province) {
                    if marked(&member, next) && !marked(&seen, next) {
                        mark(&mut seen, next);
                        component.push(self.ids[next]);
                        stack.push(next);
                    }
                }
            }
            component.sort_unstable();
            out.push(component);
        }
        out
    }

    fn row(&self, ordinal: usize) -> &[usize] {
        &self.flat[self.rows[ordinal]..self.rows[ordinal + 1]]
    }

    /// Distinct adjacent province pairs the graph holds, for reporting.
    pub fn edge_count(&self) -> usize {
        self.edges
    }
}

/// The game content the name layers need, read once per document.
pub struct NameContent {
    id: u64,
    pub graph: ProvinceGraph,
    pub bounds: HashMap<u32, ProvinceBounds>,
    /// Sea and lake provinces: the ones a fitted baseline crosses without its
    /// run ending (`names::best_run`).
    pub bridged: HashSet<u32>,
    pub localisation: Arc<Localisation>,
}

impl NameContent {
    pub fn projection_id(&self) -> u64 {
        self.id
    }
    /// Reads the adjacency rows and the localisation in `language`, and scans the bounds.
    pub fn load(layers: &ResolvedLayers, map: &MapData, language: &str) -> io::Result<Self> {
        Self::load_with_localisation(layers, map, || {
            localisation::read(layers, language).map(Arc::new)
        })
    }

    pub fn load_with_localisation(
        layers: &ResolvedLayers,
        map: &MapData,
        localisation: impl FnOnce() -> io::Result<Arc<Localisation>>,
    ) -> io::Result<Self> {
        let rows = adjacencies::read(layers)?;
        let graph = ProvinceGraph::build(map.width, map.height, &map.province_ids, &rows)?;
        let bounds = names::scan_province_bounds(&ProvinceMap {
            width: map.width,
            height: map.height,
            ids: &map.province_ids,
        });
        let bridged = map
            .definitions
            .values()
            .filter(|definition| definition.kind != ProvinceKind::Land)
            .map(|definition| definition.id)
            .collect();
        Ok(Self {
            id: super::next_projection_id(),
            graph,
            bounds,
            bridged,
            localisation: localisation()?,
        })
    }
}

/// Everything the placement reads that does not change within an hour.
pub struct PlacementInputs<'a> {
    pub map: &'a MapData,
    pub heightmap: &'a IndexedImage,
    pub content: &'a NameContent,
    pub font: &'a text::Font,
}

/// Places the country names for one decoded hour; with `groups`, grouped countries carry their
/// group's name instead.
pub fn place(
    inputs: &PlacementInputs<'_>,
    frame: &DecodedFrame,
    groups: Option<&Result<GroupLabels, String>>,
) -> io::Result<Placements> {
    let areas = NameAreas::read(inputs.map, &inputs.content.localisation, frame, groups)?;
    Ok(place_areas(inputs, &areas, None))
}

/// Everything [`place`] reads from an hour: each country that controls land,
/// its tag, the name it resolves to (or why it shows its tag) and the provinces
/// it controls. Two hours with equal areas place the same names.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NameAreas {
    countries: Vec<CountryArea>,
    warnings: Vec<String>,
    /// The areas named close in, below the country-name cutoff.
    near: Option<Box<NameAreas>>,
}

#[derive(Clone, Debug, PartialEq)]
struct CountryArea {
    tag: String,
    text: Result<String, String>,
    members: BTreeSet<u32>,
    /// A group's area (a faction's or a player's), which shows no tag when its name does not
    /// resolve.
    faction: bool,
    strategic: bool,
}

impl NameAreas {
    /// The strategic-name layer groups every province kind by strategic region;
    /// connected components and name fitting follow the ordinary map-name pass.
    pub fn strategic(
        map: &MapData,
        regions: &BTreeMap<u32, installed_content_regions::regions::StrategicRegion>,
        name: impl Fn(&installed_content_regions::regions::StrategicRegion) -> Result<String, String>,
    ) -> Self {
        Self {
            countries: regions
                .values()
                .map(|region| CountryArea {
                    tag: format!("strategic region {}", region.id),
                    text: name(region),
                    members: region
                        .provinces
                        .iter()
                        .copied()
                        .filter(|id| map.definitions.contains_key(id))
                        .collect(),
                    faction: true,
                    strategic: true,
                })
                .collect(),
            ..Default::default()
        }
    }

    /// The hour's name areas. With `groups`, a group's countries form one area under the
    /// group's name: a faction's members in the factions mode, a player's country in the
    /// players mode. Countries outside every group keep their own names when the groups say so,
    /// and carry none otherwise. Groups the recording cannot read leave no names at all.
    pub fn read(
        map: &MapData,
        localisation: &Localisation,
        frame: &DecodedFrame,
        groups: Option<&Result<GroupLabels, String>>,
    ) -> io::Result<Self> {
        let provinces = core::<tables::Provinces>(frame)?;
        let links = core::<tables::CountryLinks>(frame)?
            .iter()
            .map(|link| (link.id, link.canonical_slot))
            .collect::<BTreeMap<_, _>>();
        let countries = core::<tables::Countries>(frame)?
            .iter()
            .map(|country| (country.canonical_slot, country))
            .collect::<BTreeMap<_, _>>();
        let mut by_country: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
        for province in provinces {
            if !map.definitions.contains_key(&province.id) {
                continue;
            }
            // Positive controllers admit owned lakes to the component and regression statistics.
            if let Some(slot) = links
                .get(&province.controller)
                .filter(|_| province.controller > 0)
            {
                by_country.entry(*slot).or_default().insert(province.id);
            }
        }
        let names = CountryNames::read(frame, localisation);
        let warnings = Vec::new();
        let groups = match groups {
            Some(Ok(labels)) => Some(labels),
            // Names the recording cannot group are withheld, not replaced with the countries'.
            Some(Err(why)) => {
                return Ok(Self {
                    countries: Vec::new(),
                    warnings: vec![format!("The map mode's group names are refused: {why}")],
                    near: None,
                });
            }
            None => None,
        };
        let mut faction_areas: BTreeMap<u64, CountryArea> = BTreeMap::new();
        let mut areas = Vec::new();
        for (slot, members) in by_country {
            if let Some((faction, text)) = groups.and_then(|labels| labels.groups.get(&slot)) {
                faction_areas
                    .entry(*faction)
                    .or_insert_with(|| CountryArea {
                        tag: format!("group {faction}"),
                        text: text.clone(),
                        members: BTreeSet::new(),
                        faction: true,
                        strategic: false,
                    })
                    .members
                    .extend(members);
                continue;
            }
            if groups.is_some_and(|labels| !labels.others_named) {
                continue;
            }
            let Some(country) = countries.get(&slot) else {
                continue;
            };
            areas.push(CountryArea {
                tag: String::from_utf8_lossy(&country.tag).into_owned(),
                // The game draws its stored name in its own language;
                // the viewer rebuilds it in the reader's.
                text: names.name(slot),
                members,
                faction: false,
                strategic: false,
            });
        }
        areas.extend(faction_areas.into_values());
        Ok(Self {
            countries: areas,
            warnings,
            near: None,
        })
    }

    /// The hour's state areas: each state's provinces on the map, under the state's name. A
    /// state whose name does not resolve carries none.
    pub fn states(
        map: &MapData,
        frame: &DecodedFrame,
        name: impl Fn(&replay_data::rows::StateRow) -> Result<String, String>,
    ) -> io::Result<Self> {
        let mut members: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
        for province in core::<tables::Provinces>(frame)? {
            if province.state != 0 && map.definitions.contains_key(&province.id) {
                members
                    .entry(province.state)
                    .or_default()
                    .insert(province.id);
            }
        }
        let countries = core::<tables::States>(frame)?
            .iter()
            .filter_map(|state| {
                Some(CountryArea {
                    tag: format!("state {}", state.id),
                    text: name(state),
                    members: members.remove(&state.id)?,
                    faction: true,
                    strategic: false,
                })
            })
            .collect();
        Ok(Self {
            countries,
            warnings: Vec::new(),
            near: None,
        })
    }

    /// These areas far out, and `near` close in.
    pub fn with_near(self, near: NameAreas) -> Self {
        Self {
            near: Some(Box::new(near)),
            ..self
        }
    }
}

/// Names already placed for one country's connected area, so an hour whose
/// fronts moved re-places only the areas that changed. Placement is a pure
/// function of the tag, the text and the area, so a hit is the same name.
///
/// Holds the areas the last placement showed: each placement keeps only what it read.
#[derive(Default)]
pub struct PlacedAreas {
    areas: Mutex<Areas>,
}

type AreaKey = (String, String, Vec<u32>, bool);

#[derive(Default)]
struct Areas {
    placed: HashMap<AreaKey, PlacedArea>,
    /// Counts placements; an area's `used` is the placement that last read it.
    generation: u64,
}

struct PlacedArea {
    name: Option<WorldName>,
    used: u64,
}

impl PlacedAreas {
    fn lock(&self) -> MutexGuard<'_, Areas> {
        self.areas
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Starts a placement: the areas it reads count as used by it.
    fn begin(&self) -> u64 {
        let mut areas = self.lock();
        areas.generation += 1;
        areas.generation
    }

    /// Ends placement `generation`: what it did not read is no longer shown.
    fn end(&self, generation: u64) {
        self.lock().placed.retain(|_, area| area.used >= generation);
    }

    /// The name of one area, placed by `build` unless the last placement has it.
    fn place(
        &self,
        generation: u64,
        key: AreaKey,
        build: impl FnOnce() -> Option<WorldName>,
    ) -> Option<WorldName> {
        if let Some(area) = self.lock().placed.get_mut(&key) {
            area.used = area.used.max(generation);
            return area.name.clone();
        }
        let name = build();
        self.lock().placed.insert(
            key,
            PlacedArea {
                name: name.clone(),
                used: generation,
            },
        );
        name
    }
}

/// Places the names of `areas`, reusing `placed` areas when given.
pub fn place_areas(
    inputs: &PlacementInputs<'_>,
    areas: &NameAreas,
    placed: Option<&PlacedAreas>,
) -> Placements {
    let generation = placed.map(PlacedAreas::begin);
    let result = place_in(inputs, areas, placed, generation);
    if let (Some(placed), Some(generation)) = (placed, generation) {
        placed.end(generation);
    }
    result
}

/// [`place_areas`] within one placement `generation` of `placed`.
fn place_in(
    inputs: &PlacementInputs<'_>,
    areas: &NameAreas,
    placed: Option<&PlacedAreas>,
    generation: Option<u64>,
) -> Placements {
    let mut result = Placements {
        generation: next_generation(),
        ..Placements::default()
    };
    let mut unplaced: Vec<String> = Vec::new();
    let mut unnamed: Vec<String> = Vec::new();
    result.warnings.extend(areas.warnings.iter().cloned());
    let mut unnamed_factions: Vec<String> = Vec::new();
    for CountryArea {
        tag,
        text,
        members,
        faction,
        strategic,
    } in &areas.countries
    {
        let text = match area_text(tag, text, *faction) {
            AreaText::Shown(text) => text,
            AreaText::Tag(tag, why) => {
                unnamed.push(why);
                tag
            }
            AreaText::Withheld(why) => {
                unnamed_factions.push(why);
                continue;
            }
        };
        for component in inputs.content.graph.components(members) {
            let name = match (placed, generation) {
                (Some(placed), Some(generation)) => placed.place(
                    generation,
                    (tag.clone(), text.to_owned(), component.clone(), *strategic),
                    || world_name(inputs, tag, text, &component, *strategic),
                ),
                _ => world_name(inputs, tag, text, &component, *strategic),
            };
            match name {
                Some(name) => result.country.push(name),
                None => unplaced.push(unplaced_line(tag, &component)),
            }
        }
    }
    result
        .warnings
        .extend(placement_warnings(&unnamed, &unnamed_factions, unplaced));
    if let Some(near) = &areas.near {
        let near = place_in(inputs, near, placed, generation);
        result.warnings.extend(near.warnings.iter().cloned());
        result.near = Some(std::sync::Arc::new(near));
    }
    result
}

/// What one area shows: its name, its tag in place of a name it cannot draw, or nothing.
pub enum AreaText<'a> {
    Shown(&'a str),
    /// The tag, with the line saying why.
    Tag(&'a str, String),
    /// A group whose name does not resolve shows none, with the line saying why.
    Withheld(String),
}

/// What the area of `tag` named `text` shows; `faction` for a group's area.
pub fn area_text<'a>(tag: &'a str, text: &'a Result<String, String>, faction: bool) -> AreaText<'a> {
    match text {
        Err(reason) if faction => AreaText::Withheld(format!("{tag}: {reason}")),
        Err(reason) => AreaText::Tag(tag, format!("{tag}: {reason}")),
        Ok(text) => match text::unsupported_control(text) {
            None => AreaText::Shown(text),
            Some(control) => AreaText::Tag(
                tag,
                format!("{tag}: its name holds {control}, which map text does not draw"),
            ),
        },
    }
}

/// The line naming a connected area that carries no name.
pub fn unplaced_line(tag: &str, component: &[u32]) -> String {
    format!("{tag} ({} provinces)", component.len())
}

/// The lines a placement adds after its areas' own warnings, in their order.
pub fn placement_warnings(
    unnamed: &[String],
    unnamed_factions: &[String],
    mut unplaced: Vec<String>,
) -> Vec<String> {
    let mut warnings = Vec::new();
    if !unnamed.is_empty() {
        warnings.push(format!(
            "{} country names show their tag ({}{})",
            unnamed.len(),
            unnamed
                .iter()
                .take(8)
                .cloned()
                .collect::<Vec<_>>()
                .join("; "),
            if unnamed.len() > 8 { "; …" } else { "" }
        ));
    }
    if !unnamed_factions.is_empty() {
        warnings.push(format!(
            "{} group names are not drawn ({})",
            unnamed_factions.len(),
            unnamed_factions.join("; ")
        ));
    }
    if !unplaced.is_empty() {
        unplaced.sort();
        warnings.push(format!(
            "{} country areas carry no name because the engine's placement has no line to fit ({}{})",
            unplaced.len(),
            unplaced.iter().take(8).cloned().collect::<Vec<_>>().join(", "),
            if unplaced.len() > 8 { ", …" } else { "" }
        ));
    }
    warnings
}

/// One connected area's name in world space, or none where no line fits.
pub fn place_component(
    inputs: &PlacementInputs<'_>,
    key: &str,
    text: &str,
    area: &[u32],
    strategic: bool,
) -> Option<WorldName> {
    world_name(inputs, key, text, area, strategic)
}

/// Places one area and converts the result to world space.
fn world_name(
    inputs: &PlacementInputs<'_>,
    key: &str,
    text: &str,
    area: &[u32],
    strategic: bool,
) -> Option<WorldName> {
    if text.is_empty() {
        return None;
    }
    let map = inputs.map;
    let placed: PlacedName = names::place_name(
        &NameInput {
            text: &text::strip_color_codes(text),
            provinces: ProvinceMap {
                width: map.width,
                height: map.height,
                ids: &map.province_ids,
            },
            area,
            bridged: &inputs.content.bridged,
            bounds: Some(&inputs.content.bounds),
        },
        inputs.font,
    )?;
    let mut corners = Vec::with_capacity(placed.glyphs.len() * 4);
    let mut uvs = Vec::with_capacity(placed.glyphs.len());
    let mut pages = Vec::with_capacity(placed.glyphs.len());
    for glyph in &placed.glyphs {
        pages.push(inputs.font.page_of(glyph.ch)?);
        let mut heights = glyph
            .corners
            .map(|p| height_at(inputs.heightmap, p[0], p[1]));
        // Remove the diagonal component while preserving mean and both slopes. Do not clamp mixed
        // heights.
        let saddle = (heights[0] - heights[1] + heights[2] - heights[3]) * 0.25;
        heights[0] -= saddle;
        heights[1] += saddle;
        heights[2] -= saddle;
        heights[3] += saddle;
        for (corner, height) in glyph.corners.into_iter().zip(heights) {
            let (x, z) = world_from_pixel(map.height, corner[0], corner[1]);
            corners.push([x, height, z]);
        }
        uvs.push(glyph.uv);
    }
    if uvs.is_empty() {
        return None;
    }
    // `world_from_pixel` flips y, so the pixel-space min and max of y become
    // the world-space max and min of z.
    let (_, z_of_min_y) = world_from_pixel(map.height, 0.0, placed.aabb[1]);
    let (_, z_of_max_y) = world_from_pixel(map.height, 0.0, placed.aabb[3]);
    Some(WorldName {
        strategic_provinces: strategic.then(|| Arc::from(area)),
        key: key.to_owned(),
        text: text.to_owned(),
        pages,
        corners,
        uvs,
        aabb: [placed.aabb[0], z_of_max_y, placed.aabb[2], z_of_min_y],
        wrapped: placed.aabb[2] >= MapSize::of(inputs.map).x(),
        font_size: placed.font_size,
        spaces: placed.spaces_inserted,
        alpha: 1.0,
    })
}

/// Bilinear height on the bottom-up raster at (x-.5,z-.5),
/// clamped up to the water plane before the quad's planar projection.
fn height_at(heightmap: &IndexedImage, x: f32, y: f32) -> f32 {
    if heightmap.width < 2 || heightmap.height < 2 {
        return WATER_HEIGHT;
    }
    let x = x - 0.5;
    let z = heightmap.height as f32 - 1.0 - y - 0.5;
    let column = (x as i32).clamp(0, heightmap.width as i32 - 2) as usize;
    let row = (z as i32).clamp(0, heightmap.height as i32 - 2) as usize;
    let sample = |dx, dz| {
        heightmap.indices
            [(heightmap.height as usize - 1 - row - dz) * heightmap.width as usize + column + dx]
            as f32
    };
    let fx = x - column as f32;
    let fz = z - row as f32;
    let low = sample(0, 0) + (sample(1, 0) - sample(0, 0)) * fx;
    let high = sample(0, 1) + (sample(1, 1) - sample(0, 1)) * fx;
    ((low + (high - low) * fz) * HEIGHT_SCALE).max(WATER_HEIGHT)
}

/// One placed name's static geometry, located in the vertex chunks.
#[derive(Clone, Debug, PartialEq)]
struct StaticName {
    strategic_provinces: Option<Arc<[u32]>>,
    source: usize,
    page: usize,
    chunk: usize,
    vertices: std::ops::Range<u32>,
    wrapped: bool,
    /// Every glyph corner's `[min, max]` world box, for view culling.
    bounds: [[f32; 3]; 2],
    /// [`WorldName::alpha`].
    alpha: f32,
}

/// Builds every name's six vertices per glyph once, split into chunks of at
/// most `max_vertices` vertices without splitting a name. A name whose glyphs
/// span several font pages becomes one entry per run of equal pages, since a
/// draw binds one page.
fn static_names(
    placements: &Placements,
    max_vertices: u64,
) -> (Vec<Vec<NameVertex>>, Vec<StaticName>) {
    let mut chunks: Vec<Vec<NameVertex>> = vec![Vec::new()];
    let mut names = Vec::with_capacity(placements.country.len());
    for (source, name) in placements.country.iter().enumerate() {
        let count = name.uvs.len() as u64 * 6;
        let current = chunks.last().map_or(0, Vec::len) as u64;
        if current > 0 && current + count > max_vertices {
            chunks.push(Vec::new());
        }
        let chunk = chunks.len() - 1;
        let out = chunks.last_mut().expect("one chunk");
        name_glyphs(name, source, chunk, out, &mut names);
    }
    (chunks, names)
}

/// One name's six vertices a glyph appended to `out`, and an entry a run of equal font pages.
fn name_glyphs(
    name: &WorldName,
    source: usize,
    chunk: usize,
    out: &mut Vec<NameVertex>,
    names: &mut Vec<StaticName>,
) {
    let mut glyph = 0;
    while glyph < name.uvs.len() {
        let page = name.pages[glyph];
        let start = out.len() as u32;
        let mut bounds = [[f32::MAX; 3], [f32::MIN; 3]];
        while glyph < name.uvs.len() && name.pages[glyph] == page {
            let uv = name.uvs[glyph];
            let corner = |slot: usize, u: f32, v: f32| NameVertex {
                position: name.corners[glyph * 4 + slot],
                uv: [u, v],
            };
            let (u0, v0, u1, v1) = (uv[0], uv[1], uv[2], uv[3]);
            out.extend([
                corner(0, u0, v0),
                corner(1, u1, v0),
                corner(2, u1, v1),
                corner(0, u0, v0),
                corner(2, u1, v1),
                corner(3, u0, v1),
            ]);
            for position in &name.corners[glyph * 4..glyph * 4 + 4] {
                for axis in 0..3 {
                    bounds[0][axis] = bounds[0][axis].min(position[axis]);
                    bounds[1][axis] = bounds[1][axis].max(position[axis]);
                }
            }
            glyph += 1;
        }
        names.push(StaticName {
            strategic_provinces: name.strategic_provinces.clone(),
            source,
            page,
            chunk,
            vertices: start..out.len() as u32,
            wrapped: name.wrapped,
            bounds,
            alpha: name.alpha,
        });
    }
}

/// The glyph order a pass draws: BL, BR, TL, TR, BR, TL, vertices 3 and 4 exchanged while bending
/// country glyphs.
fn drawn_order(vertices: &[NameVertex], country_layer: bool) -> Vec<NameVertex> {
    let order = if country_layer {
        [5, 2, 0, 2, 1, 0]
    } else {
        [5, 2, 0, 1, 2, 0]
    };
    vertices
        .chunks_exact(6)
        .flat_map(|glyph| order.map(|index| glyph[index]))
        .collect()
}

/// A key's names in the draw order of the names of every key below it, and per key its glyph
/// buffer: names kept by the caller's key, each key written alone.
#[derive(Default)]
struct Keyed {
    /// The generation the keyed names began at, and the newest written.
    begun: u64,
    generation: u64,
    /// Per key held, its buffer's index in `buffers`.
    keys: std::collections::BTreeMap<u64, usize>,
    buffers: Vec<Option<wgpu::Buffer>>,
    free: Vec<usize>,
}

/// The first [`StaticName::source`] a keyed name of `key` takes; its names follow in order.
fn keyed_source(key: u64) -> usize {
    (key as usize) << 24
}

/// Native camera-view order, with one admitted view owning the whole name.
#[derive(Clone, Debug, PartialEq)]
struct NativeDraw {
    chunk: usize,
    page: usize,
    vertices: std::ops::Range<u32>,
    view: usize,
    alpha_offset: [f32; 2],
}

#[cfg(test)]
fn native_copies(
    names: &[StaticName],
    camera: Camera,
    viewport: [u32; 2],
    layer_alpha: f32,
    wraps: &[f32],
) -> Vec<NativeDraw> {
    native_hover_copies(
        names,
        camera,
        viewport,
        layer_alpha,
        wraps,
        StrategicNames::All,
    )
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StrategicNames {
    #[default]
    All,
    Hover(Option<u32>),
}

fn native_hover_copies(
    names: &[StaticName],
    camera: Camera,
    viewport: [u32; 2],
    layer_alpha: f32,
    wraps: &[f32],
    strategic_names: StrategicNames,
) -> Vec<NativeDraw> {
    if layer_alpha == 0.0 {
        return Vec::new();
    }
    let mut seen = HashSet::new();
    let mut draws = Vec::new();
    for (view, wrap) in wraps.iter().copied().enumerate() {
        let mut camera = camera;
        camera.center[0] -= wrap;
        // Test the complete glyph boxes against this view's expanded planes.
        let planes = side_planes(camera, viewport);
        for group in names.chunk_by(|a, b| a.source == b.source) {
            let first = &group[0];
            if let Some(provinces) = &first.strategic_provinces
                && let StrategicNames::Hover(province) = strategic_names
                && province.is_none_or(|province| provinces.binary_search(&province).is_err())
            {
                continue;
            }
            if seen.contains(&first.source) {
                continue;
            }
            let mut bounds = first.bounds;
            // Bound every final 20-byte glyph vertex, then subtract map width from both wrapped X endpoints.
            for name in &group[1..] {
                for (row, values) in bounds.iter_mut().enumerate() {
                    for (axis, value) in values.iter_mut().enumerate() {
                        *value = if row == 0 {
                            value.min(name.bounds[row][axis])
                        } else {
                            value.max(name.bounds[row][axis])
                        };
                    }
                }
            }
            // Test both boxes; the wrapped box wins if both survive.
            let offset = if first.wrapped && admits_box(bounds, -camera.map.x(), &planes) {
                -camera.map.x()
            } else if admits_box(bounds, 0., &planes) {
                0.
            } else {
                continue;
            };
            // Country layer passes FLT_MAX as the squared draw distance.
            let alpha = names::label_alpha(layer_alpha) * first.alpha;
            for name in group {
                draws.push(NativeDraw {
                    chunk: name.chunk,
                    page: name.page,
                    vertices: name.vertices.clone(),
                    view,
                    alpha_offset: [alpha, offset],
                });
            }
            seen.insert(first.source);
        }
    }
    draws
}

/// Collect arrows, then draws each label once with
/// alpha and zero X offset. Glyph bounds do not participate in admission.
fn arrow_draws(names: &[StaticName], admitted: &[(usize, usize)]) -> Vec<NativeDraw> {
    admitted
        .iter()
        .flat_map(|(view, source)| {
            names
                .iter()
                .filter(move |name| name.source == *source)
                .map(|name| NativeDraw {
                    chunk: name.chunk,
                    page: name.page,
                    vertices: name.vertices.clone(),
                    view: *view,
                    alpha_offset: [name.alpha, 0.],
                })
        })
        .collect()
}

/// The content shader, font pages and placed country names or order labels.
pub struct NamePass {
    strategic_names: StrategicNames,
    country_layer: bool,
    compiled: Result<super::name_shader::Names, String>,
    pages: Vec<shader_pipeline::gpu::TextureBinding>,
    chunks: Vec<wgpu::Buffer>,
    /// The names held by key instead of [`Self::chunks`]' whole placements.
    keyed: Option<Keyed>,
    draws: Vec<NativeDraw>,
    names: Vec<StaticName>,
    /// [`Placements::generation`] the chunks were built from.
    generation: Option<u64>,
    /// The names the chunks hold; a new generation of equal names keeps them.
    built: Option<Vec<WorldName>>,
    count: u32,
    /// [`Self::upload`] builds the glyph buffers its placements lack: chosen by renders that
    /// install no hours ([`Self::build_in_upload`]).
    builds_in_upload: bool,
    /// Why the frame's names are refused: placements nothing prepared before it.
    unprepared: Option<String>,
    /// The flight record's section for this pass, and what last prepared its glyph buffers: an
    /// install, a draw (placements with no names, or a render that builds in upload), or keys.
    section: &'static str,
    prepared_by: &'static str,
}

impl NamePass {
    pub fn new(
        device: &wgpu::Device,
        samples: u32,
        textures: &IconTextures,
        format: wgpu::TextureFormat,
        effect: &Arc<shader_pipeline::library::Effect>,
        arrow_technique: Option<&str>,
    ) -> io::Result<Self> {
        let country_layer = arrow_technique.is_none();
        let compiled = if country_layer {
            super::name_shader::Names::new(device, effect, samples, format, DEPTH_FORMAT)
        } else {
            super::name_shader::Names::with_technique(
                device,
                effect,
                samples,
                format,
                DEPTH_FORMAT,
                arrow_technique,
            )
        }
        .map_err(|error| error.to_string());
        Ok(Self {
            strategic_names: StrategicNames::All,
            country_layer,
            compiled,
            pages: textures.views(),
            chunks: Vec::new(),
            keyed: None,
            draws: Vec::new(),
            names: Vec::new(),
            generation: None,
            built: None,
            count: 0,
            builds_in_upload: false,
            unprepared: None,
            section: if country_layer { "names far" } else { "plan labels" },
            prepared_by: "nothing",
        })
    }

    /// Whether [`Self::upload`] builds glyph buffers itself, for renders that install no hours.
    /// Otherwise the hour's install prepares them ([`Self::prepare_static`]), and placements it
    /// did not prepare are refused by name.
    pub fn build_in_upload(&mut self, builds: bool) {
        self.builds_in_upload = builds;
    }

    /// The flight record's section for this pass (the near country names').
    pub fn record_as(&mut self, section: &'static str) {
        self.section = section;
    }

    /// What prepared the glyph buffers last was an hour's install.
    pub fn prepared_by_install(&mut self) {
        self.prepared_by = "install";
    }

    /// Whether the glyph buffers hold `placements`' names.
    fn holds(&self, placements: &Placements) -> bool {
        self.generation == Some(placements.generation)
            || self.keyed.as_ref().is_some_and(|keyed| {
                placements.country.is_empty()
                    && (keyed.begun..=keyed.generation).contains(&placements.generation)
            })
            || self.built.as_ref() == Some(&placements.country)
    }

    pub fn set_strategic_names(&mut self, selection: StrategicNames) {
        self.strategic_names = selection;
    }

    /// Uploads glyphs once per placement and selects visible country names.
    /// Order labels use their arrow's admission via [`Self::select_arrows`].
    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        placements: &Placements,
        camera: Camera,
        viewport: [u32; 2],
        layer_alpha: f32,
        wrap: &[f32],
    ) {
        self.draws.clear();
        self.unprepared = None;
        let held = self.holds(placements);
        if viewer_foundation::view_record::on() {
            let section = self.section;
            viewer_foundation::view_record::set(section, "handed_generation", placements.generation);
            viewer_foundation::view_record::set(section, "handed_names", placements.country.len() as u64);
            viewer_foundation::view_record::set(section, "held_generation", self.generation.unwrap_or(0));
            viewer_foundation::view_record::add(section, if held { "frames_held" } else { "frames_not_held" }, 1);
        }
        if held || self.builds_in_upload {
            if !held {
                self.prepared_by = "draw";
            }
            self.prepare_static(device, placements);
        } else if placements.country.is_empty() {
            // No names to draw, and the buffers keep what an install prepared: a frame handed none
            // (the names layer away composing) must not unprepare the names handed again after it.
            self.update_count();
            return;
        } else {
            let reason = format!(
                "{} reached their frame without their glyph buffers prepared (handed generation {} of {} names; the buffers hold generation {:?}, keyed {:?}, {} names built, prepared last by {})",
                if self.country_layer {
                    "country names"
                } else {
                    "plan labels"
                },
                placements.generation,
                placements.country.len(),
                self.generation,
                self.keyed.as_ref().map(|keyed| keyed.begun..=keyed.generation),
                self.built.as_ref().map_or(0, Vec::len),
                self.prepared_by,
            );
            #[cfg(any(test, feature = "test-support"))]
            panic!("{reason}");
            #[cfg(not(any(test, feature = "test-support")))]
            {
                self.unprepared = Some(reason);
                self.update_count();
                return;
            }
        }
        if self.country_layer {
            self.draws = native_hover_copies(
                &self.names,
                camera,
                viewport,
                layer_alpha,
                wrap,
                self.strategic_names,
            );
        }
        self.update_count();
    }

    /// Builds the glyph buffers of `placements`, which no camera changes, unless they hold them.
    pub fn prepare_static(&mut self, device: &wgpu::Device, placements: &Placements) {
        if self.keyed.is_some() && self.holds(placements) {
            return;
        }
        if self.generation != Some(placements.generation)
            && self.built.as_ref() != Some(&placements.country)
        {
            self.keyed = None;
            let limit = device.limits().max_buffer_size;
            let max_vertices = (limit / std::mem::size_of::<NameVertex>() as u64).max(6);
            let (chunks, names) = static_names(placements, max_vertices);
            self.chunks = chunks
                .iter()
                .filter(|chunk| !chunk.is_empty())
                .map(|chunk| {
                    let vertices = drawn_order(chunk, self.country_layer);
                    shader_pipeline::gpu::stats::create_buffer_init(
                        &device,
                        &wgpu::util::BufferInitDescriptor {
                            label: Some("native mapname vertices"),
                            contents: bytemuck::cast_slice(&vertices),
                            usage: wgpu::BufferUsages::VERTEX,
                        },
                    )
                })
                .collect();
            self.names = names;
            self.built = Some(placements.country.clone());
        }
        self.generation = Some(placements.generation);
    }

    /// Whether its glyph buffers hold `placements`' names: what [`Self::upload`] asks before it
    /// draws them.
    pub fn holds_placements(&self, placements: &Placements) -> bool {
        placements.country.is_empty() || self.holds(placements)
    }

    /// The generation of the names the pass holds by key ([`Self::set_keyed`]); none while it
    /// holds whole placements.
    pub fn keyed_generation(&self) -> Option<u64> {
        self.keyed.as_ref().map(|keyed| keyed.generation)
    }

    /// Drops every name held, whole or by key: the pass then holds keyed names, none yet, as
    /// placements of `generation`.
    pub fn begin_keyed(&mut self, generation: u64) {
        self.chunks.clear();
        self.names.clear();
        self.draws.clear();
        self.built = None;
        self.keyed = Some(Keyed { begun: generation, generation, ..Keyed::default() });
        self.generation = Some(generation);
        self.prepared_by = "keys";
        self.update_count();
    }

    /// `key`'s names (none: it is dropped) written alone, drawn after every lower key's and before
    /// every higher's, as a whole [`Self::prepare_static`] of placements listing them in key order
    /// draws them; the pass then holds placements of `generation`. Only after
    /// [`Self::begin_keyed`]; a key below 2^39 holds fewer than 2^24 names.
    pub fn set_keyed(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        generation: u64,
        key: u64,
        names: &[&WorldName],
    ) -> io::Result<()> {
        let Some(keyed) = self.keyed.as_mut() else {
            return Err(invalid("country names set by key before they were begun by key"));
        };
        if names.len() >= 1 << 24 || key >= 1 << 39 {
            return Err(invalid(format!("country names of key {key} beyond what a key holds")));
        }
        let first = keyed_source(key);
        let start = self.names.partition_point(|name| name.source < first);
        let end = self.names.partition_point(|name| name.source < keyed_source(key + 1));
        self.names.drain(start..end);
        // A key's buffer is kept when it lets go of it, and written again by the next key that fits.
        if let Some(buffer) = keyed.keys.remove(&key) {
            keyed.free.push(buffer);
        }
        if !names.is_empty() {
            let buffer = keyed.free.pop().unwrap_or_else(|| {
                keyed.buffers.push(None);
                keyed.buffers.len() - 1
            });
            let (mut vertices, mut placed) = (Vec::new(), Vec::new());
            for (at, name) in names.iter().enumerate() {
                name_glyphs(name, first + at, buffer, &mut vertices, &mut placed);
            }
            if !vertices.is_empty() {
                let limit = device.limits().max_buffer_size;
                if (vertices.len() * std::mem::size_of::<NameVertex>()) as u64 > limit {
                    keyed.free.push(buffer);
                    return Err(invalid(format!("country names of key {key} overflow one glyph buffer")));
                }
                let vertices = drawn_order(&vertices, self.country_layer);
                let bytes: &[u8] = bytemuck::cast_slice(&vertices);
                let held = &mut keyed.buffers[buffer];
                if held.as_ref().is_none_or(|held| held.size() < bytes.len() as u64) {
                    *held = Some(device.create_buffer(
                        &wgpu::BufferDescriptor {
                            label: Some("native mapname vertices of one key"),
                            size: (bytes.len() as u64).next_power_of_two().min(limit).max(bytes.len() as u64),
                            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                            mapped_at_creation: false,
                        },
                    ));
                }
                if let Some(held) = held.as_ref() {
                    shader_pipeline::gpu::stats::write_buffer(queue, held, 0, bytes);
                }
                keyed.keys.insert(key, buffer);
                self.names.splice(start..start, placed);
            } else {
                keyed.free.push(buffer);
            }
        }
        keyed.generation = generation;
        self.generation = Some(generation);
        Ok(())
    }

    /// The vertex buffer a draw reads.
    fn chunk(&self, chunk: usize) -> io::Result<&wgpu::Buffer> {
        match &self.keyed {
            Some(keyed) => keyed.buffers.get(chunk).and_then(Option::as_ref),
            None => self.chunks.get(chunk),
        }
        .ok_or_else(|| invalid(format!("map name glyph buffer {chunk} unavailable")))
    }

    pub fn select_arrows(&mut self, admitted: &[(usize, usize)]) {
        self.draws = arrow_draws(&self.names, admitted);
        self.update_count();
    }

    fn update_count(&mut self) {
        self.count = self
            .draws
            .iter()
            .map(|draw| u64::from(draw.vertices.end - draw.vertices.start))
            .sum::<u64>()
            .min(u64::from(u32::MAX)) as u32;
    }

    pub fn validate_draw(&self, frames: &[&[u8]], view: Option<usize>) -> io::Result<()> {
        if let Some(reason) = &self.unprepared {
            return Err(io::Error::other(reason.clone()));
        }
        for draw in self
            .draws
            .iter()
            .filter(|draw| view.is_none_or(|view| draw.view == view))
        {
            self.pages
                .get(draw.page)
                .ok_or_else(|| invalid(format!("map name font page {} unavailable", draw.page)))?;
            frames
                .get(draw.view)
                .ok_or_else(|| invalid("map name camera view unavailable"))?;
        }
        Ok(())
    }

    /// Blends country names over the world passes already in the target.
    pub fn draw(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        frames: &[&[u8]],
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
    ) -> io::Result<()> {
        self.draw_view(arena, frames, encoder, targets, None, None)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_view(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        frames: &[&[u8]],
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
        view: Option<usize>,
        retained: Option<&RefCell<super::shader_scene::State>>,
    ) -> io::Result<()> {
        self.validate_draw(frames, view)?;
        let compiled = viewer_foundation::map::gpu_contract::compiled(&self.compiled)?;
        let draws: Vec<_> = self
            .draws
            .iter()
            .filter(|draw| view.is_none_or(|view| draw.view == view))
            .collect();
        if draws.is_empty() {
            return Ok(());
        }
        let bindings: Vec<_> = draws
            .iter()
            .map(|draw| {
                let page = &self.pages[draw.page];
                let frame = frames[draw.view];
                if let Some(retained) = retained {
                    compiled.retain(&mut retained.borrow_mut(), frame, draw.alpha_offset, page);
                }
                compiled.bindings(arena, frame, draw.alpha_offset, page)
            })
            .collect::<io::Result<_>>()?;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("compiled map names"),
            color_attachments: &[Some(targets.scene_attachment(wgpu::LoadOp::Load))],
            depth_stencil_attachment: Some(targets.depth_attachment(wgpu::LoadOp::Load)),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        compiled.begin(&mut pass);
        let buffers = draws
            .iter()
            .map(|draw| self.chunk(draw.chunk))
            .collect::<io::Result<Vec<_>>>()?;
        let mut previous = None;
        for ((draw, binding), buffer) in draws.iter().zip(&bindings).zip(buffers) {
            binding.bind_after(&mut pass, previous);
            previous = Some(binding);
            shader_pipeline::gpu::stats::state();
            pass.set_vertex_buffer(0, buffer.slice(..));
            shader_pipeline::gpu::stats::draw();
            pass.draw(draw.vertices.clone(), 0..1);
        }
        Ok(())
    }

    /// Vertices the last upload drew, every copy counted, for reporting.
    pub fn vertex_count(&self) -> u32 {
        self.count
    }
}

/// The country layer's fade.
///
/// the engine walks the alpha by the frame delta towards 1 at
/// or above `NGraphics.DRAW_COUNTRY_NAMES_CUTOFF` and 0 below it, so the
/// transition takes one second. A headless render has no frame history, so
/// [`LayerFade::instant`] jumps straight to the target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayerFade(pub f32);

impl LayerFade {
    fn target(camera: Camera) -> f32 {
        if camera.height >= camera.defines.country_names_cutoff {
            1.0
        } else {
            0.0
        }
    }

    /// The settled value at this camera.
    pub fn instant(camera: Camera) -> Self {
        Self(Self::target(camera))
    }

    /// A changed hovered region resets the already stepped fade, unless the local preference
    /// requests all names.
    pub fn strategic_hover(
        self,
        show_all: bool,
        picked: Option<u32>,
        previous: Option<u32>,
    ) -> Self {
        if !show_all && picked.is_some() && picked != previous {
            Self(0.0)
        } else {
            self
        }
    }

    /// One frame of the engine's integration.
    pub fn step(self, camera: Camera, delta: f32) -> Self {
        let (value, target, delta) = (self.0, Self::target(camera), delta.max(0.0));
        Self(if target > value {
            (value + delta).min(target)
        } else {
            (value - delta).max(target)
        })
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests;
