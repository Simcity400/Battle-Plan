//! Each state's resource icon in the resources map mode: the content's `resource_mapicon`
//! window (map icon type 3) with one `resource_mapicon_item` row per resource in its
//! `resources_list`, from the hour's recorded resource origins
//! ([`viewer_foundation::resource_origins`]).
//!
//! The icons exist only in the resources mode ([`Visibility::resource_icons`]) and strictly below
//! `NGraphics.UNITS_ICONS_DISTANCE_CUTOFF`; their rows and the window's flag show strictly below
//! `NGraphics.RESOURCE_MAP_ICON_TEXT_CUTOFF`. They sort by `NMapIcons.<table>_PRIORITY_RESOURCE`.
//! Every state of every country qualifies ([`Hour::has_icon`]); fog, intel and the viewing
//! country play no part.
//!
//! A row is shown for each resource of a positive nominal amount, in the resource database's
//! order. Its text is the resource strip's inline icon at the resource's `icon_frame`, then
//! `MAP_ICON_NAME` with `NAME` the resource's `PRODUCTION_MATERIALS_<RESOURCE>` and `VAL` the
//! value: `MAP_RESOURCE_ICON_VAL` (`VAL` delivered, `MAX` nominal, each rounded half away from
//! zero) in red below the nominal amount or below zero, green above it, uncoloured at it, then
//! a line break. The window's flag is the rights holder's when every row's rights go to the one
//! country other than the controller; otherwise each row whose rights go elsewhere shows its
//! holder's flag. A flag icon is bound to its country's canonical slot, the flag index a masked
//! shield draws through the GUI resources' flag source.
//!
//! The window stands at the member centroid nearest the rounded mean of the state's province
//! centroids, with horizontal wrapping and authored province order breaking ties.

use std::{
    collections::{BTreeMap, HashMap},
    io,
};

use installed_content_core::{defines::Defines, gui::native};
use installed_content_localisation::{localisation::Argument as Arg, localisation::Localisation};
use replay_frame::frame::DecodedFrame;
use viewer_foundation::{
    map::{
        camera::Camera,
        gui_resources::{count_noted, count_refused, viewport as gui_resources_viewport},
        icon_windows::{Owner, Plan, PoolView, Retry},
        mode_visibility::{Priorities, PriorityIcon, Visibility},
        native_gui::{
            self,
            lifecycle::{Instance, MapIconPool, SlotId},
        },
        native_resources::Context,
        native_text::{COLOR, ICON},
        native_windows::NativeWindows,
        sprites::{self, Batch},
    },
    resource_origins::{self, Definition, Hour, RowColour, StateIcon, whole},
};

/// The state icon's window and its row window.
pub use viewer_foundation::map::native_windows::{
    RESOURCE_ITEM_WINDOW as ITEM_WINDOW, RESOURCE_WINDOW as WINDOW,
};
const LIST: &str = "resources_list";
const FLAG: &str = "flag";
const DESC: &str = "desc";
/// The sprite each row's inline icon names (`GFX_` is tried first, as for every inline icon).
const STRIP: &str = "resources_strip";
/// How the manager anchors the window at its projected point: centred on both axes.
const ALIGNMENT: u8 = 0x12;

/// The `NDefines` operands of the resource icons.
#[derive(Clone, Debug, PartialEq)]
pub struct ResourceIconDefines {
    /// `NGraphics.UNITS_ICONS_DISTANCE_CUTOFF`, a fixed5 define: the icons exist strictly below
    /// it.
    pub icons_cutoff: f32,
    /// `NGraphics.RESOURCE_MAP_ICON_TEXT_CUTOFF`, an integer define compared as a float: rows and
    /// the window's flag show strictly below it.
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

/// What the icons read of the recorded content stack.
pub struct ResourceIconContent {
    pub origins: resource_origins::Content,
    pub defines: ResourceIconDefines,
}

impl ResourceIconContent {
    pub fn load(layers: &installed_content_core::ResolvedLayers, defines: &Defines) -> io::Result<Self> {
        Ok(Self {
            origins: resource_origins::Content::load(layers, defines)?,
            defines: ResourceIconDefines::load(defines)?,
        })
    }

    /// What the content's resource files misread.
    pub fn warnings(&self) -> &[String] {
        &self.origins.warnings
    }
}

/// The inline sprite the rows' text draws, for the windows' resource load; the windows' own
/// sprites load with the other map-icon windows ([`NativeWindows::assets`]).
pub fn gui_assets() -> native_gui::Assets {
    let mut assets = native_gui::Assets::default();
    assets.add_text(&format!("{ICON}{STRIP}|1 "));
    assets
}

/// Everything one frame of resource icons needs besides the decoded hour.
pub struct ResourceIconInputs<'a> {
    pub content: &'a ResourceIconContent,
    pub localisation: &'a Localisation,
    pub state_provinces: &'a BTreeMap<u32, Vec<u32>>,
    /// Integer bitmap centroids, with sampled terrain height.
    pub centroid: &'a dyn Fn(u32) -> Option<[f32; 3]>,
    pub camera: Camera,
    /// The frame in device pixels, which the projection uses.
    pub viewport: [u32; 2],
    /// Device pixels per interface pixel.
    pub ui_scale: f32,
    pub visibility: Visibility,
}

/// What one frame of resource icons drew, and what it could not.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResourceIconReport {
    pub icons: usize,
    pub rows: usize,
    /// What is not drawn, by reason, with how often each was met.
    pub refused: BTreeMap<String, usize>,
}

impl ResourceIconReport {
    fn refuse(&mut self, reason: String) {
        *self.refused.entry(reason).or_default() += 1;
    }
}

/// One row's text and its own flag (the rights holder's canonical slot).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub text: String,
    pub flag: Option<u32>,
}

struct Placed {
    state: u32,
    screen: [f32; 2],
    depth: f32,
    priority: i8,
    /// Whether the rows and the window's flag show (below the text cutoff).
    text: bool,
    rows: Vec<Row>,
    flag: Option<u32>,
}

/// A prepared hour's icons. Preparing never changes a presented window.
#[derive(Default)]
pub struct PreparedResourceIcons {
    placed: Vec<Placed>,
    pub report: ResourceIconReport,
}

/// Whether a mode at camera `height` has resource icons, and whether their rows show.
pub fn shown(visibility: &Visibility, height: f32, defines: &ResourceIconDefines) -> Option<bool> {
    (visibility.resource_icons && height < defines.icons_cutoff)
        .then_some(height < defines.text_cutoff)
}

/// One row's text, as the game builds it.
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

/// The state's rows' text and flags.
fn rows(
    hour: &Hour<'_>,
    localisation: &Localisation,
    icon: &StateIcon,
) -> Result<Vec<Row>, String> {
    icon.rows
        .iter()
        .map(|row| {
            Ok(Row {
                text: row_text(
                    localisation,
                    &hour.definitions()[row.definition],
                    row.delivered,
                    row.nominal,
                )?,
                flag: row.flag.map(|raw| hour.slot(raw)).transpose()?,
            })
        })
        .collect()
}

/// Nearest member centroid to the rounded, equally weighted centroid mean. Ties retain
/// authored member order; victory points and unit-stack placements do not participate.
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
                viewer_foundation::map::unit_pose::unwrap(x, first[0], width)
            });
            points.push([x, z]);
        }
        if points.is_empty() {
            continue;
        }
        let mean = [0, 1].map(|axis| {
            (points.iter().map(|p| p[axis]).sum::<f32>() / points.len() as f32).round()
        });
        let distance = |p: [f32; 2]| (p[0] - mean[0]).powi(2) + (p[1] - mean[1]).powi(2);
        let mut best = 0;
        for i in 1..points.len() {
            if distance(points[i]) < distance(points[best]) {
                best = i;
            }
        }
        anchors.insert(state, provinces[best]);
    }
    Ok(anchors)
}

/// One hour's resource icons, placed for the camera.
pub fn prepare(inputs: &ResourceIconInputs<'_>, frame: &DecodedFrame) -> PreparedResourceIcons {
    let mut prepared = PreparedResourceIcons::default();
    let defines = &inputs.content.defines;
    let Some(text) = shown(&inputs.visibility, inputs.camera.height, defines) else {
        return prepared;
    };
    let hour = match Hour::new(frame, &inputs.content.origins) {
        Ok(hour) => hour,
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
    let states: Vec<u32> = hour.states().collect();
    for state in states {
        let icon = match hour.icon(state) {
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
        let Some(projection) = inputs
            .camera
            .project_map_icon(world, inputs.viewport, extent)
        else {
            continue;
        };
        let laid = if text {
            rows(&hour, inputs.localisation, &icon)
        } else {
            Ok(Vec::new())
        };
        let (rows, flag) = match laid
            .and_then(|rows| Ok((rows, icon.flag.map(|raw| hour.slot(raw)).transpose()?)))
        {
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
            priority,
            text,
            rows,
            flag,
        });
    }
    prepared.placed.sort_by(|a, b| b.depth.total_cmp(&a.depth));
    prepared
}

/// The pooled windows and each pooled window's row windows of one map view.
#[derive(Debug, Default)]
pub struct ResourceIconView {
    pool: PoolView,
    /// A pooled window's rows, in list order; a row window outlives the rows that leave it
    /// hidden, as the game's list keeps its items.
    items: HashMap<SlotId, Vec<Instance>>,
}

impl ResourceIconView {
    pub fn retry(&self) -> Option<Retry> {
        self.pool.retry()
    }

    pub fn take_running(&mut self) -> bool {
        self.pool.take_running()
    }
}

/// The controls the code binds.
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

/// Showing a control clears its hide bit before `Show`; hiding calls `Hide` before setting it.
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

/// Shows `element` with `country`'s flag (its canonical slot), or hides it.
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

/// The frame's update of one live icon: the list and the window's flag shown or hidden, and a
/// row window for each row, its text and its flag written.
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
    // The rows fill the list's items in order, each shown as it is filled; the items the rows
    // leave stay hidden.
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

/// The first `count` row windows laid out in the list box's holder of `layout`, placed where the
/// list box puts its rows. The number of rows placed.
#[allow(clippy::too_many_arguments)] // The window's layout, its rows and their resources.
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
    /// Presents the icons through the view's pool and draws them onto `batch` in depth order.
    /// The flags draw through `context`'s flag source.
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
                // The window's flag starts hidden.
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
        report
    }
}

#[cfg(test)]
mod tests;
