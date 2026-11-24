
use std::{
    collections::{BTreeMap, HashMap},
    io,
};

use content_reader_defines::defines::Defines;
use content_reader_gui::gui::native;
use content_reader_localisation::{localisation::Argument as Arg, localisation::Localisation};
use replay_frame::frame::DecodedFrame;
use viewer_gpu::{camera::Camera, icon_tracks::{IconAnchor, IconFamily}};
use viewer_native_gui::{
    gui_resources::{count_noted, count_refused, viewport as gui_resources_viewport},
    icon_windows::{Owner, Plan, PoolView, Retry},
    native_gui::{self, lifecycle::{Instance, MapIconPool, SlotId}},
    native_resources::Context,
    native_text::{COLOR, ICON},
    native_windows::NativeWindows,
    sprites::{self, Batch},
};
use viewer_foundation::mode_visibility::{Priorities, PriorityIcon, Visibility};
use viewer_runtime::resource_origins::{self, Definition, Hour, RowColour, Values, whole};

use viewer_native_gui::native_windows::{
    RESOURCE_ITEM_WINDOW as ITEM_WINDOW,
    RESOURCE_WINDOW as WINDOW,
};
const LIST: &str = "resources_list";
const FLAG: &str = "flag";
const DESC: &str = "desc";
const STRIP: &str = "resources_strip";
const ALIGNMENT: u8 = 0x12;

#[derive(Clone, Debug, PartialEq)]
pub struct ResourceIconDefines {
    pub icons_cutoff: f32,
    pub text_cutoff: f32,
    pub priorities: Priorities,
}

impl ResourceIconDefines {
    pub fn load(defines: &Defines) -> io::Result<Self> {
        Ok(Self {
            icons_cutoff: (defines.fixed5("NGraphics", "UNITS_ICONS_DISTANCE_CUTOFF")? as f64
                / 100_000.0) as f32,
            text_cutoff: defines.int32("NGraphics", "RESOURCE_MAP_ICON_TEXT_CUTOFF")? as f32,
            priorities: Priorities::load(defines)?,
        })
    }
}

pub struct ResourceIconContent {
    pub origins: resource_origins::Content,
    pub defines: ResourceIconDefines,
}

impl ResourceIconContent {
    pub fn load(layers: &content_reader_layers::ResolvedLayers, defines: &Defines) -> io::Result<Self> {
        Ok(Self {
            origins: resource_origins::Content::load(layers, defines)?,
            defines: ResourceIconDefines::load(defines)?,
        })
    }

    pub fn warnings(&self) -> &[String] {
        &self.origins.warnings
    }
}

pub fn gui_assets() -> native_gui::Assets {
    let mut assets = native_gui::Assets::default();
    assets.add_text(&format!("{ICON}{STRIP}|1 "));
    assets
}

pub struct ResourceIconInputs<'a> {
    pub content: &'a ResourceIconContent,
    pub localisation: &'a Localisation,
    pub state_provinces: &'a BTreeMap<u32, Vec<u32>>,
    pub centroid: &'a dyn Fn(u32) -> Option<[f32; 3]>,
    pub camera: Camera,
    pub viewport: [u32; 2],
    pub ui_scale: f32,
    pub visibility: Visibility,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResourceIconReport {
    pub icons: usize,
    pub rows: usize,
    pub refused: BTreeMap<String, usize>,
}

impl ResourceIconReport {
    fn refuse(&mut self, reason: String) {
        *self.refused.entry(reason).or_default() += 1;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub text: String,
    pub flag: Option<u32>,
}

struct Placed {
    state: u32,
    screen: [f32; 2],
    depth: f32,
    anchor: IconAnchor,
    priority: i8,
    text: bool,
    rows: Vec<Row>,
    flag: Option<u32>,
}

#[derive(Default)]
pub struct PreparedResourceIcons {
    placed: Vec<Placed>,
    pub report: ResourceIconReport,
}

pub fn shown(
    visibility: &Visibility,
    distance: f32,
    defines: &ResourceIconDefines,
) -> Option<bool> {
    (visibility.resource_icons && distance < defines.icons_cutoff)
        .then_some(distance < defines.text_cutoff)
}

pub fn row_text(
    localisation: &Localisation,
    definition: &Definition,
    delivered: i64,
    nominal: i64,
) -> Result<String, String> {
    let frame = definition.icon_frame.ok_or_else(|| {
        format!(
            "resource {}'s icon_frame is not a positive whole number, which the content reader does not keep",
            definition.name
        )
    })?;
    let localise = |key: &str, args: &[(&str, Arg<'_>)]| {
        localisation
            .localise_with(key.as_bytes(), args)
            .map_err(|error| error.to_string())
    };
    let amounts = localise(
        "MAP_RESOURCE_ICON_VAL",
        &[
            ("VAL", Arg::Int32(whole(delivered))),
            ("MAX", Arg::Int32(whole(nominal))),
        ],
    )?;
    let colour = RowColour::of(delivered, nominal)
        .code()
        .map(|code| format!("{COLOR}{code}"))
        .unwrap_or_default();
    let value = format!("{colour}{amounts}{COLOR}!");
    let name = localise(&definition.name_key(), &[])?;
    let line = localise(
        "MAP_ICON_NAME",
        &[("NAME", Arg::Text(&name)), ("VAL", Arg::Text(&value))],
    )?;
    Ok(format!("{ICON}{STRIP}|{frame} {line}\n"))
}

pub fn anchor_provinces(
    members: &BTreeMap<u32, Vec<u32>>,
    centroid: &dyn Fn(u32) -> Option<[f32; 3]>,
    width: f32,
) -> Result<BTreeMap<u32, u32>, String> {
    let mut anchors = BTreeMap::new();
    for (&state, provinces) in members {
        let mut points = Vec::new();
        for &province in provinces {
            let [x, _, z] = centroid(province).ok_or_else(|| {
                format!("resource icon state {state}: province {province} has no centroid")
            })?;
            let x = points.first().map_or(x, |first: &[f32; 2]| {
                unwrap_anchor_x(x as i32, first[0] as i32, width as i32) as f32
            });
            points.push([x, z]);
        }
        if points.is_empty() {
            continue;
        }
        let mut mean =
            [0, 1].map(|axis| points.iter().map(|p| p[axis]).sum::<f32>() / points.len() as f32);
        mean[0] = normalize_anchor_x(mean[0], width);
        let mean = mean.map(|value| (value + 0.5) as i32);
        let distance = |p: [f32; 2]| {
            let dx = (normalize_anchor_x(p[0], width) as i32).wrapping_sub(mean[0]);
            let dz = (p[1] as i32).wrapping_sub(mean[1]);
            dx.wrapping_mul(dx).wrapping_add(dz.wrapping_mul(dz))
        };
        let mut best = 0;
        for i in 1..points.len() {
            if distance(points[best]) < 0 || distance(points[i]) < distance(points[best]) {
                best = i;
            }
        }
        anchors.insert(state, provinces[best]);
    }
    Ok(anchors)
}

fn unwrap_anchor_x(mut x: i32, reference: i32, width: i32) -> i32 {
    while reference.wrapping_sub(x).wrapping_abs() > width / 2 {
        x = x.wrapping_add(if reference.wrapping_sub(x) < 0 {
            width.wrapping_neg()
        } else {
            width
        });
    }
    x
}

fn normalize_anchor_x(x: f32, width: f32) -> f32 {
    if x < 0.0 {
        x + width
    } else if x >= width {
        x - width
    } else {
        x
    }
}

pub fn prepare(inputs: &ResourceIconInputs<'_>, frame: &DecodedFrame) -> PreparedResourceIcons {
    let defines = &inputs.content.defines;
    if !inputs.visibility.resource_icons
        || (inputs.camera.free().is_none()
            && shown(&inputs.visibility, inputs.camera.height, defines).is_none())
    {
        return PreparedResourceIcons::default();
    }
    lay_out(inputs, &hour_icons(frame, &inputs.content.origins, inputs.localisation))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Laid {
    pub rows: Result<Vec<Row>, String>,
    pub flag: Result<Option<u32>, String>,
}

pub type HourIcons = Result<BTreeMap<u32, Result<Option<Laid>, String>>, String>;

pub fn hour_icons(frame: &DecodedFrame, origins: &resource_origins::Content, localisation: &Localisation) -> HourIcons {
    let hour = Hour::new(frame, origins)?;
    Ok(hour.states().map(|state| (state, laid(&hour, localisation, state))).collect())
}

pub fn laid(values: &dyn Values, localisation: &Localisation, state: u32) -> Result<Option<Laid>, String> {
    Ok(resource_origins::icon(values, state)?.map(|icon| laid_icon(values, localisation, &icon)))
}

pub fn laid_icon(values: &dyn Values, localisation: &Localisation, icon: &resource_origins::StateIcon) -> Laid {
    let slot = |raw: u32| values.slot(raw).ok_or_else(|| format!("CountryLinks lacks {raw}"));
    let rows = icon
        .rows
        .iter()
        .map(|row| {
            Ok(Row {
                text: row_text(localisation, &values.definitions()[row.definition], row.delivered, row.nominal)?,
                flag: row.flag.map(slot).transpose()?,
            })
        })
        .collect();
    Laid { rows, flag: icon.flag.map(slot).transpose() }
}

pub fn lay_out(inputs: &ResourceIconInputs<'_>, icons: &HourIcons) -> PreparedResourceIcons {
    let mut prepared = PreparedResourceIcons::default();
    let defines = &inputs.content.defines;
    if !inputs.visibility.resource_icons
        || (inputs.camera.free().is_none()
            && shown(&inputs.visibility, inputs.camera.height, defines).is_none())
    {
        return prepared;
    }
    let icons = match icons {
        Ok(icons) => icons,
        Err(why) => {
            prepared
                .report
                .refuse(format!("{WINDOW} is not drawn: {why}"));
            return prepared;
        }
    };
    let anchors = match anchor_provinces(
        inputs.state_provinces,
        inputs.centroid,
        inputs.camera.map.width as f32,
    ) {
        Ok(anchors) => anchors,
        Err(why) => {
            prepared
                .report
                .refuse(format!("{WINDOW} is not drawn: {why}"));
            return prepared;
        }
    };
    let extent = gui_resources_viewport(inputs.viewport, inputs.ui_scale);
    let priority = defines
        .priorities
        .get(inputs.visibility.priorities, PriorityIcon::Resource);
    for (&state, icon) in icons {
        let icon = match icon {
            Ok(Some(icon)) => icon,
            Ok(None) => continue,
            Err(why) => {
                prepared
                    .report
                    .refuse(format!("{WINDOW} is not drawn: {why}"));
                continue;
            }
        };
        let Some(world) = anchors
            .get(&state)
            .and_then(|&province| (inputs.centroid)(province))
        else {
            prepared.report.refuse(format!(
                "{WINDOW} is not drawn: its state has no province with a map-icon anchor"
            ));
            continue;
        };
        let Some(text) = shown(
            &inputs.visibility,
            inputs.camera.icon_distance(world),
            defines,
        ) else {
            continue;
        };
        let Some(projection) = inputs
            .camera
            .project_map_icon(world, inputs.viewport, extent)
        else {
            continue;
        };
        let laid = if text { icon.rows.clone() } else { Ok(Vec::new()) };
        let (rows, flag) = match laid.and_then(|rows| Ok((rows, icon.flag.clone()?))) {
            Ok(laid) => laid,
            Err(why) => {
                prepared
                    .report
                    .refuse(format!("{WINDOW} is not drawn: {why}"));
                continue;
            }
        };
        prepared.placed.push(Placed {
            state,
            screen: projection.position,
            depth: projection.depth,
            anchor: IconAnchor {
                world,
                cutoff: defines.icons_cutoff,
                family: IconFamily::Resources,
                moves: None,
            },
            priority,
            text,
            rows,
            flag,
        });
    }
    prepared.placed.sort_by(|a, b| b.depth.total_cmp(&a.depth));
    prepared
}

#[derive(Debug, Default)]
pub struct ResourceIconView {
    pool: PoolView,
    items: HashMap<SlotId, Vec<Instance>>,
}

impl ResourceIconView {
    pub fn running_visibly(&self) -> bool {
        self.pool.running_visibly()
    }

    pub fn retry(&self) -> Option<Retry> {
        self.pool.retry()
    }

    pub fn take_running(&mut self) -> bool {
        self.pool.take_running()
    }
}

struct Controls<'w> {
    list: &'w native::Element,
    flag: &'w native::Element,
    item: &'w native::Element,
    desc: &'w native::Element,
    item_flag: &'w native::Element,
}

fn named<'w>(
    window: &'w native::Element,
    kind: native::Kind,
    name: &str,
) -> Result<&'w native::Element, String> {
    window
        .find(kind, name.as_bytes())
        .ok_or_else(|| format!("{} has no {name}", window.path))
}

fn controls(windows: &NativeWindows) -> Result<Controls<'_>, String> {
    let root = windows
        .gui_type(WINDOW.as_bytes())
        .ok_or_else(|| format!("no GUI template {WINDOW} is registered"))?;
    let item = windows
        .gui_type(ITEM_WINDOW.as_bytes())
        .ok_or_else(|| format!("no GUI template {ITEM_WINDOW} is registered"))?;
    Ok(Controls {
        list: named(root, native::Kind::ListBox, LIST)?,
        flag: named(root, native::Kind::Icon, FLAG)?,
        desc: named(item, native::Kind::TextBox, DESC)?,
        item_flag: named(item, native::Kind::Icon, FLAG)?,
        item,
    })
}

fn set_shown(
    instance: &mut Instance,
    window: &native::Element,
    element: &native::Element,
    shown: bool,
) -> Result<(), String> {
    if shown {
        instance.set_hide_flag(window, element, false)?;
        instance.show(window, element)
    } else {
        instance.hide(window, element)?;
        instance.set_hide_flag(window, element, true)
    }
}

fn bind_flag(
    instance: &mut Instance,
    window: &native::Element,
    element: &native::Element,
    country: Option<u32>,
) -> Result<(), String> {
    match country {
        Some(slot) => {
            instance.control(window, element)?.shield = Some(
                i32::try_from(slot).map_err(|_| format!("country slot {slot} is no flag index"))?,
            );
            set_shown(instance, window, element, true)
        }
        None => set_shown(instance, window, element, false),
    }
}

fn update(
    instance: &mut Instance,
    root: &native::Element,
    controls: &Controls<'_>,
    icon: &Placed,
    entries: &mut Vec<Instance>,
    viewport: [i32; 2],
    resources: &dyn native_gui::Resources,
) -> Result<(), String> {
    set_shown(instance, root, controls.list, icon.text)?;
    bind_flag(
        instance,
        root,
        controls.flag,
        icon.flag.filter(|_| icon.text),
    )?;
    if !icon.text {
        return Ok(());
    }
    for (index, row) in icon.rows.iter().enumerate() {
        if entries.len() <= index {
            entries.push(Instance::construct(controls.item, viewport, resources)?);
        }
        let entry = &mut entries[index];
        entry.gui_show(controls.item, controls.item)?;
        entry.set_text(controls.item, controls.desc, &row.text, resources)?;
        bind_flag(entry, controls.item, controls.item_flag, row.flag)?;
    }
    for entry in entries.iter_mut().skip(icon.rows.len()) {
        entry.hide(controls.item, controls.item)?;
    }
    Ok(())
}

#[expect(clippy::too_many_arguments, reason = "the layout, controls and entries, the viewport, the resources, and the parts and notes it fills")]
fn row_parts(
    layout: &native_gui::Layout,
    controls: &Controls<'_>,
    entries: &[Instance],
    count: usize,
    viewport: [i32; 2],
    resources: &dyn native_gui::Resources,
    parts: &mut [Vec<native_gui::Part>],
    notes: &mut BTreeMap<String, usize>,
) -> Result<usize, native_gui::Refusals> {
    let Some(at) = layout
        .holders
        .iter()
        .position(|holder| holder.path == controls.list.path)
    else {
        return Ok(0);
    };
    let mut laid = Vec::with_capacity(count);
    for entry in entries.iter().take(count) {
        if let Some(item) = entry.layout(controls.item, resources, None)? {
            count_noted(notes, ITEM_WINDOW, &item.diagnostics);
            laid.push(item);
        }
    }
    let sizes: Vec<[i32; 2]> = laid.iter().map(|item| item.size).collect();
    let mut bindings = native_gui::Bindings::default();
    bindings.viewport = Some(viewport);
    let rows =
        native_gui::widgets::list_box_rows(controls.list, &sizes, None, resources, &bindings)?;
    count_noted(notes, WINDOW, &rows.diagnostics);
    let holder = &layout.holders[at];
    let mut placed = 0;
    for (place, item) in rows.places.iter().zip(&laid) {
        if let Some(place) = place {
            parts[at].extend(holder.place(*place, item));
            placed += 1;
        }
    }
    parts[at].extend(holder.place_scrollbar(&rows));
    Ok(placed)
}

impl PreparedResourceIcons {
    #[cfg(any(test, feature = "test-support"))]
    pub fn placed_positions(&self) -> Vec<[f32; 2]> {
        self.placed.iter().map(|item| item.screen).collect()
    }

    pub fn draw(
        &self,
        context: &Context<'_, impl native_gui::Resources + native_gui::Sheets>,
        view: &mut ResourceIconView,
        batch: &mut Batch,
    ) -> ResourceIconReport {
        let mut report = self.report.clone();
        if !view.pool.same(context) {
            view.items.clear();
        }
        let owners: Vec<_> = self
            .placed
            .iter()
            .map(|icon| Owner {
                id: icon.state,
                screen: icon.screen,
                dirty: false,
            })
            .collect();
        let placed: HashMap<u32, (usize, &Placed)> = self
            .placed
            .iter()
            .enumerate()
            .map(|(ordinal, icon)| (icon.state, (ordinal, icon)))
            .collect();
        let bound = controls(context.windows);
        let ResourceIconView { pool, items } = view;
        let mut slots: HashMap<u32, SlotId> = HashMap::new();
        let drawn = pool.present(
            context,
            &Plan {
                name: WINDOW,
                kind: MapIconPool::Resource,
                alignment: ALIGNMENT,
                owners: &owners,
            },
            Box::new(|instance, root| {
                let flag = named(root, native::Kind::Icon, FLAG)?;
                set_shown(instance, root, flag, false)
            }),
            |owner, slot, instance, root| {
                let controls = bound.as_ref().map_err(Clone::clone)?;
                let (_, icon) = placed[&owner];
                slots.insert(owner, slot);
                update(
                    instance,
                    root,
                    controls,
                    icon,
                    items.entry(slot).or_default(),
                    context.viewport,
                    context.resources,
                )
            },
            &mut report.refused,
        );
        let Ok(controls) = bound.as_ref() else {
            return report;
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
            let mut parts = vec![Vec::new(); drawn.layout.holders.len()];
            let mut rows = 0;
            if icon.text && !icon.rows.is_empty() {
                let entries = slots
                    .get(&icon.state)
                    .and_then(|slot| items.get(slot))
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                match row_parts(
                    &drawn.layout,
                    controls,
                    entries,
                    icon.rows.len(),
                    context.viewport,
                    context.resources,
                    &mut parts,
                    &mut report.refused,
                ) {
                    Ok(placed) => rows = placed,
                    Err(refusals) => {
                        count_refused(&mut report.refused, ITEM_WINDOW, &refusals);
                        continue;
                    }
                }
            }
            batch.at_layer(sprites::layer(icon.priority, ordinal as u32, 0));
            batch.at_anchor(Some(icon.anchor));
            match native_gui::draw_composed(
                &drawn.layout,
                &parts,
                drawn.origin,
                context.resources,
                batch,
            ) {
                Ok(()) => {
                    report.icons += 1;
                    report.rows += rows;
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
