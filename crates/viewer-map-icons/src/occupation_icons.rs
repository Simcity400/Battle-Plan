use viewer_map_modes::map_mode::ColorList;
use super::resource_icons::anchor_provinces;
use content_reader_defines::defines::Defines;
use content_reader_gui::gui::native;
use content_reader_politics::occupation_modifiers;
use replay_data::{rows, rows::Recorded, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::{
    collections::{BTreeMap, HashMap},
    io,
};
use viewer_gpu::{camera::Camera, icon_tracks::{IconAnchor, IconFamily}};
use viewer_native_gui::{
    gui_resources::{count_noted, count_refused, viewport as gui_viewport},
    icon_windows::{self, Retry},
    native_gui::{self, HitGeometry, SpriteSetter, lifecycle::{Admission, Held, Instance, MapIconPool, Pool, Rewrote, SlotId, Unknown}},
    native_resources::Context,
    native_text::ICON,
    native_windows::NativeWindows,
    sprites::{self, Batch},
};

use viewer_native_gui::native_windows::{
    OCCUPATION_ITEM_WINDOW as ITEM_WINDOW,
    OCCUPATION_WINDOW as WINDOW,
};
const TEXT: &str = "text";
const GRID: &str = "modifier_grid_list";
const BADGE: &str = "icon";
const INCREASING: &str = "increase_reversed_texticon";
const DECREASING: &str = "decrease_reversed_texticon";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Resistance,
    Compliance,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IconDefines {
    pub cutoff: f32,
    pub badge_cutoff: f32,
    pub reenable: i64,
    pub priority: i8,
    pub resistance_colors: ColorList,
    pub compliance_colors: ColorList,
}
impl IconDefines {
    pub fn load(defines: &Defines) -> io::Result<Self> {
        let colors = |name| -> io::Result<_> {
            Ok(ColorList(
                defines
                    .numbers("NGraphics", name)?
                    .iter()
                    .map(|n| *n as f32)
                    .collect(),
            ))
        };
        Ok(Self {
            cutoff: defines.number("NGraphics", "RESISTANCE_MAP_ICON_DISTANCE_CUTOFF")? as f32,
            badge_cutoff: defines
                .number("NGraphics", "RESISTANCE_MAP_ICON_MODIFIERS_DISTANCE_CUTOFF")?
                as f32,
            reenable: defines.fixed5("NResistance", "RESISTANCE_TARGET_TO_REENABLE_RESISTANCE")?,
            priority: viewer_foundation::mode_visibility::priority(defines, "TOP_MAP_ICON")?,
            resistance_colors: colors("OCCUPATION_RESISTANCE_MAP_MODE_COLORS")?,
            compliance_colors: colors("OCCUPATION_COMPLIANCE_MAP_MODE_COLORS")?,
        })
    }
}

pub struct Content {
    pub definitions: occupation_modifiers::Definitions,
    pub defines: IconDefines,
}
impl Content {
    pub fn load(layers: &content_reader_layers::ResolvedLayers, defines: &Defines) -> io::Result<Self> {
        Ok(Self {
            definitions: occupation_modifiers::Definitions::read(layers)?,
            defines: IconDefines::load(defines)?,
        })
    }
    pub fn warnings(&self) -> impl Iterator<Item = &String> {
        self.definitions
            .notes
            .iter()
            .chain(&self.definitions.refusals)
    }
}

pub fn gui_assets(content: &Content) -> native_gui::Assets {
    let mut assets = native_gui::Assets::default();
    for definition in content.definitions.definitions.values() {
        assets.sprites.insert(definition.sprite.as_bytes().to_vec());
    }
    assets.add_text(&format!("{ICON}{INCREASING} {ICON}{DECREASING} "));
    assets
}

pub struct Inputs<'a> {
    pub content: &'a Content,
    pub mode: Option<Mode>,
    pub country: u32,
    pub state_provinces: &'a BTreeMap<u32, Vec<u32>>,
    pub centroid: &'a dyn Fn(u32) -> Option<[f32; 3]>,
    pub camera: Camera,
    pub viewport: [u32; 2],
    pub ui_scale: f32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub icons: usize,
    pub badges: usize,
    pub refused: BTreeMap<String, usize>,
}
impl Report {
    fn refuse(&mut self, why: impl Into<String>) {
        *self.refused.entry(why.into()).or_default() += 1;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Badge {
    pub token: String,
    pub sprite: String,
    pub frame: i32,
}
struct Placed {
    state: u32,
    screen: [f32; 2],
    depth: f32,
    anchor: IconAnchor,
    text: String,
    color: [f32; 4],
    badges: Vec<Badge>,
}
#[derive(Default)]
pub struct PreparedOccupationIcons {
    placed: Vec<Placed>,
    priority: i8,
    pub report: Report,
}

pub fn percentage(raw: i64, speed: i64, mode: Mode) -> String {
    let mut text = format!(
        "{}{}%",
        if raw < 0 { "-" } else { "" },
        raw.unsigned_abs() / 100_000
    );
    if mode == Mode::Resistance && speed != 0 {
        text.push_str(&format!(
            "{ICON}{} ",
            if speed < 0 { INCREASING } else { DECREASING }
        ));
    }
    text
}

fn admitted(country: u32, level: i64, compliance: i64, map: &Recorded<Option<rows::OccupationMap>>, reenable: i64) -> Result<bool, String> {
    if country as i32 <= 0 {
        return Ok(false);
    }
    if level != 0 || compliance > 0 {
        return Ok(true);
    }
    let Recorded::Value(Some(map)) = map else {
        return Err("occupation map cached target is not recorded".into());
    };
    Ok(map
        .stored_target
        .ok_or("occupation map cached target is absent")?
        >= reenable)
}

fn badges(content: &Content, tokens: &[Vec<u8>]) -> Result<Vec<Badge>, BTreeMap<String, usize>> {
    let mut definitions = Vec::with_capacity(tokens.len());
    let mut refused = BTreeMap::new();
    for token in tokens {
        let definition = (|| -> Result<_, String> {
            let token = std::str::from_utf8(token)
                .map_err(|_| "occupation map definition identity is not UTF-8")?;
            content.definitions.find(token)?.ok_or_else(|| {
                format!(
                    "occupation map active definition {token} is absent from the recorded content"
                )
            })
        })();
        match definition {
            Ok(definition) => definitions.push(definition),
            Err(why) => *refused.entry(why).or_default() += 1,
        }
    }
    if !refused.is_empty() {
        return Err(refused);
    }
    definitions.sort_by_key(|definition| definition.threshold);
    Ok(definitions
        .into_iter()
        .map(|definition| Badge {
            token: definition.token.clone(),
            sprite: definition.sprite.clone(),
            frame: definition.frame,
        })
        .collect())
}

pub fn prepare(inputs: &Inputs<'_>, frame: &DecodedFrame) -> PreparedOccupationIcons {
    if inputs.mode.is_none() || (inputs.camera.free().is_none() && inputs.camera.height >= inputs.content.defines.cutoff) {
        return PreparedOccupationIcons { priority: inputs.content.defines.priority, ..Default::default() };
    }
    lay_out(inputs, &hour_marks(frame, inputs.content))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    pub controller: u32,
    pub slot: Option<u32>,
    pub resistance: Option<Result<Option<Occupation>, String>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Occupation {
    pub level: i64,
    pub speed: i64,
    pub compliance: i64,
    pub badges: [Result<Vec<Badge>, BTreeMap<String, usize>>; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct HourMarks {
    pub marks: BTreeMap<u32, Mark>,
    pub viewers: BTreeMap<u32, u32>,
    pub resistance: Option<String>,
}

pub type Marks = Result<HourMarks, String>;

pub fn occupation(content: &Content, country: u32, level: i64, speed: i64, compliance: i64, map: &Recorded<Option<rows::OccupationMap>>) -> Result<Option<Occupation>, String> {
    if !admitted(country, level, compliance, map, content.defines.reenable)? {
        return Ok(None);
    }
    let badges = |tokens: fn(&rows::OccupationMap) -> &Vec<Vec<u8>>| match map {
        Recorded::Value(Some(map)) => badges(content, tokens(map)),
        _ => Err(BTreeMap::from([("occupation map active definitions are not recorded".into(), 1)])),
    };
    Ok(Some(Occupation { level, speed, compliance, badges: [badges(|map| &map.resistance), badges(|map| &map.compliance)] }))
}

pub fn hour_marks(frame: &DecodedFrame, content: &Content) -> Marks {
    let states = frame.complete::<tables::States>().map_err(|e| e.to_string())?;
    let links: HashMap<u32, u32> = frame.complete::<tables::CountryLinks>().map_err(|e| e.to_string())?.iter().map(|row| (row.id, row.canonical_slot)).collect();
    let mut viewers = BTreeMap::new();
    for country in frame.complete::<tables::Countries>().map_err(|e| e.to_string())? {
        viewers.entry(country.canonical_slot).or_insert(country.id);
    }
    let (resistance, by_state): (Option<String>, HashMap<u32, &rows::StateResistanceRow>) = match frame.complete::<tables::StateResistance>() {
        Ok(rows) => (None, rows.iter().map(|row| (row.state, row)).collect()),
        Err(why) => (Some(why.to_string()), HashMap::new()),
    };
    let marks = states
        .iter()
        .map(|state| {
            let resistance = by_state.get(&state.id).map(|row| occupation(content, row.country, row.level, row.speed, row.compliance, &row.map));
            (state.id, Mark { controller: state.controller, slot: links.get(&state.controller).copied(), resistance })
        })
        .collect();
    Ok(HourMarks { marks, viewers, resistance })
}

pub fn lay_out(inputs: &Inputs<'_>, marks: &Marks) -> PreparedOccupationIcons {
    let mut out = PreparedOccupationIcons {
        priority: inputs.content.defines.priority,
        ..Default::default()
    };
    let Some(mode) = inputs.mode else {
        return out;
    };
    let defines = &inputs.content.defines;
    let shown = inputs.camera.height < defines.cutoff;
    if inputs.camera.free().is_none() && !shown {
        return out;
    }
    let run = (|| -> Result<(), String> {
        let marks = marks.as_ref().map_err(Clone::clone)?;
        let viewing_raw = *marks
            .viewers
            .get(&inputs.country)
            .ok_or_else(|| format!("Countries lacks occupation viewing slot {}", inputs.country))?;
        if let Some(why) = &marks.resistance {
            return Err(why.clone());
        }
        let anchors = anchor_provinces(
            inputs.state_provinces,
            inputs.centroid,
            inputs.camera.map.width as f32,
        )?;
        let extent = gui_viewport(inputs.viewport, inputs.ui_scale);
        for (&state, mark) in &marks.marks {
            let controlled = if mark.controller == viewing_raw {
                true
            } else if mark.controller == 0 || viewing_raw == 0 {
                false
            } else {
                mark.slot.ok_or_else(|| {
                    format!(
                        "CountryLinks lacks occupation controller {}",
                        mark.controller
                    )
                })? == inputs.country
            };
            if !controlled {
                continue;
            }
            let Some(resistance) = &mark.resistance else {
                out.report.refuse(format!(
                    "{WINDOW} state {state}: StateResistance is absent"
                ));
                continue;
            };
            let row = match resistance {
                Ok(None) => continue,
                Err(why) => {
                    out.report
                        .refuse(format!("{WINDOW} state {state}: {why}"));
                    continue;
                }
                Ok(Some(row)) => row,
            };
            let world = match inputs.state_provinces.get(&state) {
                Some(members) if members.is_empty() => [0.; 3],
                Some(_) => match anchors
                    .get(&state)
                    .and_then(|&province| (inputs.centroid)(province))
                {
                    Some(world) => world,
                    None => {
                        out.report.refuse(format!(
                            "{WINDOW} state {state}: graphical-state anchor is absent"
                        ));
                        continue;
                    }
                },
                None => {
                    out.report.refuse(format!(
                        "{WINDOW} state {state}: recorded state members are absent"
                    ));
                    continue;
                }
            };
            let Some(projection) = inputs
                .camera
                .project_map_icon(world, inputs.viewport, extent)
            else {
                continue;
            };
            let distance = inputs.camera.icon_distance(world);
            if distance >= defines.cutoff {
                continue;
            }
            let active = if distance < defines.badge_cutoff {
                row.badges[usize::from(mode == Mode::Compliance)].clone()
            } else {
                Ok(Vec::new())
            };
            let active = match active {
                Ok(active) => active,
                Err(refused) => {
                    for (why, count) in refused {
                        *out.report
                            .refused
                            .entry(format!("{WINDOW} state {state} badges: {why}"))
                            .or_default() += count;
                    }
                    Vec::new()
                }
            };
            let (raw, colors) = if mode == Mode::Resistance {
                (row.level, &defines.resistance_colors)
            } else {
                (row.compliance, &defines.compliance_colors)
            };
            let mut color = colors.color(raw as f32 / 100_000.);
            color[3] = 1.0;
            out.placed.push(Placed {
                state,
                screen: projection.position,
                depth: projection.depth,
                anchor: IconAnchor {
                    world,
                    cutoff: defines.cutoff,
                    family: IconFamily::Occupation,
                    moves: None,
                },
                text: percentage(raw, row.speed, mode),
                color,
                badges: active,
            });
        }
        Ok(())
    })();
    if let Err(why) = run {
        out.placed.clear();
        out.report.refuse(format!("{WINDOW}: {why}"));
    }
    out.placed.sort_by(|a, b| b.depth.total_cmp(&a.depth));
    out
}

#[derive(Debug, Default)]
pub struct OccupationIconView {
    built: Option<(NativeWindows, [i32; 2])>,
    pool: Option<Pool<u32>>,
    entries: HashMap<SlotId, Vec<Entry>>,
    last: Option<f32>,
    active: bool,
}
#[derive(Debug)]
struct Entry {
    instance: Instance,
    definition: Option<String>,
}
impl OccupationIconView {
    pub fn running(&self) -> bool {
        self.active
    }

    pub fn retry(&self) -> Option<Retry> {
        self.pool.as_ref().and_then(Pool::retry)
    }
    pub fn take_running(&mut self) -> bool {
        std::mem::take(&mut self.active)
    }

    fn same<R>(&self, context: &Context<'_, R>) -> bool {
        self.built.as_ref().is_some_and(|(windows, viewport)| {
            windows.same(context.windows) && *viewport == context.viewport
        })
    }

    fn present(
        &mut self,
        context: &Context<'_, impl native_gui::Resources + native_gui::Sheets>,
        root: &native::Element,
        controls: &Controls<'_>,
        placed: &[Placed],
        notes: &mut BTreeMap<String, usize>,
    ) -> Result<Vec<icon_windows::Drawn>, String> {
        if !self.same(context) {
            if context.time.is_none() {
                return Ok(Vec::new());
            }
            *self = Self::default();
            self.pool = Some(Pool::new(
                root,
                context.viewport,
                MapIconPool::Occupation,
                Box::new(|_, _| Ok(())),
                context.resources,
            )?);
            self.built = Some((context.windows.clone(), context.viewport));
        }
        let pool = self
            .pool
            .as_mut()
            .expect("the matching occupation view was built");
        let owners: HashMap<_, _> = placed.iter().map(|icon| (icon.state, icon)).collect();
        let (updated, tick) = if let Some(time) = context.time {
            self.active = false;
            let size = native_gui::container_frame(root, Some(context.viewport))?.size;
            for icon in placed {
                if pool.request(icon.state).is_none() {
                    *notes
                        .entry(format!(
                            "{WINDOW} is not drawn: {}",
                            icon_windows::POOL_EXHAUSTED
                        ))
                        .or_default() += 1;
                }
            }
            let pass = pool.update(root, |owner, _| {
                owners.get(owner).map_or(Admission::Empty, |icon| {
                    let point = icon_windows::point(icon.screen, size, 0x12);
                    if icon_windows::rectangle_inside(point, size, context.viewport) {
                        Admission::Inside(point)
                    } else {
                        Admission::Outside
                    }
                })
            })?;
            for erased in pass.erased {
                if let Some(entries) = self.entries.get_mut(&erased.slot) {
                    reach_entries(entries, controls, &erased.held)?;
                }
            }
            (
                pass.updated,
                Some(icon_windows::frame(&mut self.last, time)),
            )
        } else {
            (
                pool.visible()
                    .filter(|icon| owners.contains_key(icon.owner))
                    .map(|icon| (*icon.owner, icon.slot))
                    .collect(),
                None,
            )
        };
        for (owner, slot) in updated {
            let instance = pool.get(slot).expect("updated occupation slot is live");
            let entries = self.entries.entry(slot).or_default();
            reach_entries(entries, controls, &instance.take_held())?;
            match update(
                instance,
                root,
                controls,
                owners[&owner],
                entries,
                context.viewport,
                context.resources,
            ) {
                Ok(()) => pool.rewrote(slot, Rewrote::Update),
                Err(why) => pool.refuse(slot, Unknown::Update, why.into()),
            }
        }
        if let Some(tick) = tick {
            pool.tick_clocks(root, context.resources, tick)?;
            for entries in self.entries.values_mut() {
                for entry in entries {
                    entry
                        .instance
                        .tick_clocks(controls.item, context.resources, tick)?;
                }
            }
            let mut windows: Vec<_> = pool
                .registrations()
                .map(|(at, slot)| (at, (slot, None)))
                .collect();
            windows.extend(self.entries.iter().flat_map(|(&slot, entries)| {
                entries
                    .iter()
                    .enumerate()
                    .map(move |(index, entry)| (entry.instance.registration(), (slot, Some(index))))
            }));
            windows.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
            for (_, (slot, entry)) in windows {
                if let Some(index) = entry {
                    self.entries
                        .get_mut(&slot)
                        .expect("registered occupation rows exist")[index]
                        .instance
                        .tick_transitions(controls.item)?;
                } else {
                    pool.tick_transitions_of(root, slot)?;
                    if let Some(entries) = self.entries.get_mut(&slot) {
                        reach_entries(entries, controls, &pool.take_held(slot))?;
                    }
                }
            }
            pool.tick_draws(root, context.resources)?;
            for entries in self.entries.values_mut() {
                for entry in entries {
                    entry
                        .instance
                        .tick_draws(controls.item, context.resources)?;
                }
            }
        }
        count_noted(notes, WINDOW, &pool.take_diagnostics());
        self.active |= pool.active()
            || self
                .entries
                .values()
                .flatten()
                .any(|entry| entry.instance.active());
        Ok(pool
            .visible()
            .filter(|icon| owners.contains_key(icon.owner))
            .filter_map(|icon| {
                icon_windows::visible_layout(&icon, root, WINDOW, context.resources, notes)
            })
            .collect())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hover {
    Resistance { state: u32 },
    Modifier { state: u32, token: String },
}
#[derive(Clone, Debug)]
pub struct Hit {
    pub geometry: HitGeometry,
    pub layer: u64,
    pub hover: Hover,
}
impl Hit {
    pub fn contains(&self, point: [f32; 2]) -> bool {
        self.geometry.contains(point)
    }
    pub fn bounds(&self) -> Option<[f32; 4]> {
        self.geometry.bounds()
    }
}

pub fn hit_at(hits: &[Hit], point: [f32; 2]) -> Option<&Hit> {
    let mut top: Option<&Hit> = None;
    for hit in hits.iter().filter(|hit| hit.contains(point)) {
        if top.is_none_or(|top| hit.layer >= top.layer) {
            top = Some(hit);
        }
    }
    top
}

struct Controls<'a> {
    text: &'a native::Element,
    grid: &'a native::Element,
    item: &'a native::Element,
    badge: &'a native::Element,
}
fn named<'a>(
    root: &'a native::Element,
    kind: native::Kind,
    name: &str,
) -> Result<&'a native::Element, String> {
    root.find(kind, name)
        .ok_or_else(|| format!("{} has no {name}", root.path))
}
fn controls(windows: &NativeWindows) -> Result<Controls<'_>, String> {
    let root = windows
        .gui_type(WINDOW.as_bytes())
        .ok_or_else(|| format!("no GUI template {WINDOW} is registered"))?;
    let item = windows
        .gui_type(ITEM_WINDOW.as_bytes())
        .ok_or_else(|| format!("no GUI template {ITEM_WINDOW} is registered"))?;
    Ok(Controls {
        text: named(root, native::Kind::TextBox, TEXT)?,
        grid: named(root, native::Kind::Grid, GRID)?,
        badge: named(item, native::Kind::Icon, BADGE)?,
        item,
    })
}
fn describe(refusals: &[native_gui::Refusal]) -> String {
    refusals
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

fn reach_entries(
    entries: &mut [Entry],
    controls: &Controls<'_>,
    held: &[Held],
) -> Result<(), String> {
    for held in held.iter().filter(|held| held.path == controls.grid.path) {
        for entry in &mut *entries {
            if held.shown {
                entry.instance.gui_show(controls.item, controls.item)?;
            } else {
                entry.instance.hide(controls.item, controls.item)?;
            }
        }
    }
    Ok(())
}

fn update(
    instance: &mut Instance,
    root: &native::Element,
    controls: &Controls<'_>,
    icon: &Placed,
    entries: &mut Vec<Entry>,
    viewport: [i32; 2],
    resources: &dyn native_gui::Resources,
) -> Result<(), String> {
    instance.set_text(root, controls.text, &icon.text, resources)?;
    instance.control(root, controls.text)?.color = Some(icon.color);
    for (index, badge) in icon.badges.iter().enumerate() {
        if entries.len() <= index {
            entries.push(Entry {
                instance: Instance::construct(controls.item, viewport, resources)?,
                definition: None,
            });
        }
        let entry = &mut entries[index];
        entry.instance.gui_show(controls.item, controls.item)?;
        if entry.definition.as_deref() != Some(badge.token.as_str()) {
            entry.instance.set_sprite(
                controls.item,
                controls.badge,
                badge.sprite.as_bytes(),
                SpriteSetter::Icon,
            )?;
            entry
                .instance
                .set_frame(controls.item, controls.badge, badge.frame)?;
            entry.definition = Some(badge.token.clone());
        }
    }
    for entry in entries.iter_mut().skip(icon.badges.len()) {
        entry.instance.hide(controls.item, controls.item)?;
    }
    let layout = instance
        .layout(root, resources, None)
        .map_err(|e| describe(&e))?;
    let Some(holder) = layout.as_ref().and_then(|layout| {
        layout
            .holders
            .iter()
            .find(|holder| holder.path == controls.grid.path)
    }) else {
        return Ok(());
    };
    let grid = controls
        .grid
        .grid()
        .ok_or("occupation badge list is not a grid")?;
    let (original, _, cell) = native_gui::grid_geometry(controls.grid, grid, holder.content)?;
    let count =
        native_gui::grid_slots(controls.grid, holder.content, icon.badges.len(), None)?.len();
    let count = i32::try_from(count)
        .map_err(|_| "occupation badge grid displayed count exceeds native integer range")?;
    let current = instance
        .control(root, controls.grid)?
        .position
        .unwrap_or(original);
    let centered = [
        original[0].wrapping_sub(count.wrapping_mul(cell[0]) / 2),
        current[1],
    ];
    instance.control(root, controls.grid)?.position = Some(centered);
    let placed = instance
        .finalize_grid(
            root,
            controls.grid,
            holder,
            &vec![controls.item; icon.badges.len()],
        )
        .map_err(|e| describe(&e))?;
    for (entry, place) in entries.iter_mut().zip(placed.places()) {
        entry
            .instance
            .set_position(controls.item, controls.item, place)?;
        entry
            .instance
            .set_opacity(controls.item, controls.item, holder.opacity, true)?;
    }
    instance
        .synchronize_scroll(root, root, resources, false)
        .map_err(|e| describe(&e))?;
    Ok(())
}

fn control_hit(
    layout: &native_gui::Layout,
    control: &native::Element,
    origin: [i32; 2],
) -> Option<HitGeometry> {
    layout
        .regions
        .iter()
        .find(|region| region.path == control.path)
        .and_then(|region| region.hit.as_ref())
        .filter(|geometry| geometry.form != native_gui::Hit::Through)
        .map(|geometry| geometry.shifted(origin))
}

impl PreparedOccupationIcons {
    #[cfg(any(test, feature = "test-support"))]
    pub fn placed_positions(&self) -> Vec<[f32; 2]> {
        self.placed.iter().map(|item| item.screen).collect()
    }

    pub fn draw(
        &self,
        context: &Context<'_, impl native_gui::Resources + native_gui::Sheets>,
        view: &mut OccupationIconView,
        batch: &mut Batch,
        hits: &mut Vec<Hit>,
    ) -> Report {
        hits.clear();
        let mut report = self.report.clone();
        let placed: HashMap<_, _> = self
            .placed
            .iter()
            .enumerate()
            .map(|(ordinal, icon)| (icon.state, (ordinal, icon)))
            .collect();
        let controls = match controls(context.windows) {
            Ok(controls) => controls,
            Err(why) => {
                if !self.placed.is_empty() {
                    *report
                        .refused
                        .entry(format!("{WINDOW} is not drawn: {why}"))
                        .or_default() += self.placed.len();
                }
                return report;
            }
        };
        let root = context
            .windows
            .gui_type(WINDOW.as_bytes())
            .expect("occupation controls have a parent window");
        let drawn = match view.present(context, root, &controls, &self.placed, &mut report.refused)
        {
            Ok(drawn) => drawn,
            Err(why) => {
                if !self.placed.is_empty() {
                    *report
                        .refused
                        .entry(format!("{WINDOW} is not drawn: {why}"))
                        .or_default() += self.placed.len();
                }
                return report;
            }
        };
        let mut drawn: Vec<_> = drawn
            .into_iter()
            .filter_map(|drawn| {
                placed
                    .get(&drawn.owner)
                    .map(|&(ordinal, icon)| (ordinal, icon, drawn))
            })
            .collect();
        drawn.sort_by_key(|(ordinal, ..)| *ordinal);
        for (ordinal, icon, drawn) in drawn {
            let layer = sprites::layer(self.priority, ordinal as u32, 0);
            let Some(pool) = view.pool.as_mut() else {
                continue;
            };
            let Some(slot) = pool.slot_of(&icon.state) else {
                continue;
            };
            let Some(instance) = pool.get(slot) else {
                continue;
            };
            let layout = match instance.layout_with_regions(root, context.resources, None, true) {
                Ok(Some(layout)) => layout,
                Ok(None) => continue,
                Err(why) => {
                    count_refused(&mut report.refused, WINDOW, &why);
                    continue;
                }
            };
            let mut parts = vec![Vec::new(); layout.holders.len()];
            let mut current_hits = Vec::new();
            if let Some(geometry) = control_hit(&layout, controls.text, drawn.origin) {
                current_hits.push(Hit {
                    geometry,
                    layer,
                    hover: Hover::Resistance { state: icon.state },
                });
            }
            let mut badge_count = 0;
            if let Some(at) = layout
                .holders
                .iter()
                .position(|holder| holder.path == controls.grid.path)
            {
                let holder = &layout.holders[at];
                let entries = view.entries.entry(slot).or_default();
                let displayed = native_gui::grid_slots(
                    controls.grid,
                    holder.content,
                    icon.badges.len(),
                    holder.position,
                );
                let count = match displayed {
                    Ok(places) => places.len(),
                    Err(why) => {
                        report.refuse(format!("{WINDOW}: {why}"));
                        continue;
                    }
                };
                for (index, entry) in entries.iter_mut().enumerate() {
                    let entry = &mut entry.instance;
                    if index < count
                        && let Err(why) =
                            entry.set_opacity(controls.item, controls.item, holder.opacity, true)
                    {
                        report.refuse(format!("{ITEM_WINDOW}: {why}"));
                        continue;
                    }
                    if index >= count {
                        continue;
                    }
                    match entry.layout_with_regions(controls.item, context.resources, None, true) {
                        Ok(Some(mut item)) => {
                            count_noted(&mut report.refused, ITEM_WINDOW, &item.diagnostics);
                            parts[at].extend(holder.place(entry.position(), &item));
                            item.regions = holder.place_regions(entry.position(), &item.regions);
                            if let Some(geometry) = control_hit(&item, controls.badge, drawn.origin)
                            {
                                current_hits.push(Hit {
                                    geometry,
                                    layer,
                                    hover: Hover::Modifier {
                                        state: icon.state,
                                        token: icon.badges[index].token.clone(),
                                    },
                                });
                            }
                            badge_count += 1;
                        }
                        Ok(None) => {}
                        Err(why) => count_refused(&mut report.refused, ITEM_WINDOW, &why),
                    }
                }
            }
            batch.at_layer(layer);
            batch.at_anchor(Some(icon.anchor));
            match native_gui::draw_composed(&layout, &parts, drawn.origin, context.resources, batch)
            {
                Ok(()) => {
                    report.icons += 1;
                    report.badges += badge_count;
                    hits.extend(current_hits);
                }
                Err(why) => count_refused(&mut report.refused, WINDOW, &[why]),
            }
        }
        batch.at_anchor(None);
        report
    }
}

#[cfg(test)]
mod tests;
