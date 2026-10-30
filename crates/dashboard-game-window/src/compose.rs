//! Lays a bound window and the rows its code adds to its list boxes and grid boxes out through
//! the native GUI, as the map viewer lays its windows out, into a [`Laid`] window, which is
//! painted as the game draws it into a [`Scene`] ([`crate::paint`]).
//!
//! A window's binder constructs the root [`Instance`] and one per row, applies the calls the
//! game's code makes (texts, sprites, frames, shows and hides), and hands them here with what
//! the app explains for each row on hover. List rows are placed where the list box places them
//! at the list's scroll offset ([`widgets::list_box_rows`]), its scrollbar after them; grid
//! items at the slots the code gives them, as the grid finalizes them
//! ([`native_gui::finalize_grid_positions`]); and every part is drawn in the game's attach
//! order ([`native_gui::Layout::composed_parts`]).
//!
//! A grid's enclosing container scrolls as the game's does once its code fills the grid: the
//! grid's finalized extent is kept on the instance, the container's scrollbars are refreshed
//! from its children ([`Instance::synchronize_scroll`]), and the content is moved to the
//! container's offset along each axis in `scroll` as the wheel moves it
//! ([`Instance::scroll_to`]). Each of its shown bars is then one of the scene's scrolling areas,
//! its items' hovers placed and clipped where they are drawn.

use std::collections::BTreeMap;

use installed_content_core::gui::native::{Element, Fields, Kind};
use viewer_foundation::map::{
    gui_resources::{noted_cause, refused_cause},
    native_gui::{self, Bindings, Part, Refusals, Resources, Sheets, lifecycle::Instance, widgets},
};

use crate::scene::{Counted, Scene, SceneHover, SceneList};

/// What the app explains when the pointer rests on a row: its in-game name and details.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hover {
    pub title: String,
    pub details: Vec<String>,
}

/// One row window the code adds to a list box.
pub struct Row {
    pub instance: Instance,
    pub hover: Option<Hover>,
}

/// A list box of the root and the rows the code adds to it, in order.
pub struct BoundList<'a> {
    pub list: &'a Element,
    pub template: &'a Element,
    pub rows: Vec<Row>,
}

/// A grid box of the root and the item windows the code appends to it, each at its slot.
pub struct BoundGrid<'a> {
    pub grid: &'a Element,
    pub template: &'a Element,
    /// Each item with its `[column, row]` slot, in the order the code appends them.
    pub items: Vec<(Row, [i32; 2])>,
}

/// Why a window is not drawn, each reason counted.
pub type Refused = Vec<String>;

/// What a binder hands over to be laid out: its constructed root, the rows and items its code
/// adds to the root's list and grid boxes, and what the app itself adds to the drawing.
pub struct Window<'a> {
    pub name: &'a str,
    pub root: &'a Element,
    pub instance: &'a Instance,
    pub lists: &'a [BoundList<'a>],
    pub grids: &'a [BoundGrid<'a>],
    /// Root controls the app explains on hover, by element path.
    pub hovers: &'a [(&'a str, Hover)],
}

/// What a window is painted with: its session's sprite sheets and texture pages, and the
/// session's painter, built on the first paint.
pub struct Canvas<'a> {
    pub sheets: &'a dyn Sheets,
    pub pages: &'a [installed_content_image::dds::Texture],
    pub painter: &'a mut Option<crate::paint::Painter>,
}

/// Each scrolling area's offset, by its element path and whether it scrolls across
/// ([`crate::scene::SceneList`]).
pub type Scroll = BTreeMap<(String, bool), i32>;

/// The path of the element `path` lies directly below.
fn parent(path: &str) -> Option<&str> {
    path.rsplit_once(['.', '/']).map(|(parent, _)| parent)
}

/// Lays `window` out, each list scrolled to its offset in `scroll` (clamped to its range; zero
/// when absent).
pub fn compose(
    window: &Window<'_>,
    scroll: &Scroll,
    resources: &dyn Resources,
) -> Result<Laid, Refused> {
    let Window {
        name,
        root,
        instance,
        lists,
        grids,
        hovers: control_hovers,
    } = *window;
    let refused = |refusals: Refusals| -> Refused {
        let mut counted = Counted::default();
        for refusal in &refusals {
            counted.note(refused_cause(name, refusal));
        }
        counted.lines()
    };
    let filled;
    let (instance, scrolled) = if grids.is_empty() {
        (instance, Vec::new())
    } else {
        let (instance, scrolled) = scroll_grids(window, scroll, resources, &refused)?;
        filled = instance;
        (&filled, scrolled)
    };
    let layout = instance
        .layout_with_regions(root, resources, None, true)
        .map_err(refused)?
        .ok_or_else(|| vec![format!("{name} draws nothing")])?;
    let mut notes = Counted::default();
    for diagnostic in &layout.diagnostics {
        notes.note(noted_cause(name, diagnostic));
    }
    let mut items: Vec<Vec<Part>> = vec![Vec::new(); layout.holders.len()];
    let mut hovers = Vec::new();
    let mut scenes = Vec::new();
    let mut bindings = Bindings::default();
    bindings.viewport = Some(instance.viewport());
    bindings.regions = true;
    for bound in lists {
        let path = &bound.list.path;
        let Some(at) = layout
            .holders
            .iter()
            .position(|holder| &holder.path == path)
        else {
            // A hidden list box holds no rows.
            continue;
        };
        let Fields::ListBox(list) = &bound.list.fields else {
            return Err(vec![format!("{path} is not a list box")]);
        };
        let mut laid = Vec::with_capacity(bound.rows.len());
        for row in &bound.rows {
            match row
                .instance
                .layout_with_regions(bound.template, resources, None, true)
                .map_err(refused)?
            {
                Some(item) => {
                    for diagnostic in &item.diagnostics {
                        notes.note(noted_cause(name, diagnostic));
                    }
                    laid.push((item, row.hover.clone()));
                }
                None => notes.note(format!(
                    "{name}: a {} row draws nothing",
                    bound.template.path
                )),
            }
        }
        let sizes: Vec<[i32; 2]> = laid.iter().map(|(item, ..)| item.size).collect();
        let smooth = bound.list.kind == Kind::SmoothListBox;
        // A standard list box stacks its rows down whatever its orientation.
        let horizontal = smooth && list.horizontal != 0;
        let axis = usize::from(!horizontal);
        // Where each row starts along the axis, and where the last ends.
        let starts: Vec<i32> = std::iter::once(0)
            .chain(sizes.iter().scan(0i32, |run, size| {
                *run = run.wrapping_add(size[axis]);
                Some(*run)
            }))
            .collect();
        let asked = scroll
            .get(&(path.clone(), horizontal))
            .copied()
            .unwrap_or(0)
            .max(0);
        let (rows, offset, max) = if smooth {
            // A smooth list box scrolls by pixels over the rows' run beyond its own extent.
            let run = starts[sizes.len()];
            let max = run.wrapping_sub(list.size[axis]).max(0);
            let offset = asked.min(max);
            let rows = widgets::list_box_rows(
                bound.list,
                &sizes,
                Some(offset as f32),
                resources,
                &bindings,
            )
            .map_err(refused)?;
            (rows, offset, max)
        } else {
            // A standard list box scrolls by whole rows, up to the first its bar's far end shows.
            let first = starts.iter().rposition(|start| *start <= asked).unwrap_or(0);
            let rows = widgets::list_box_rows(
                bound.list,
                &sizes,
                Some(first as f32),
                resources,
                &bindings,
            )
            .map_err(refused)?;
            let max = starts[(rows.range as usize).min(sizes.len())].max(0);
            (rows, asked.min(max), max)
        };
        for diagnostic in &rows.diagnostics {
            notes.note(noted_cause(name, diagnostic));
        }
        let holder = &layout.holders[at];
        // The pointer reaches a row only inside the list box.
        let region = layout.regions.iter().find(|region| &region.path == path);
        let list_box = region.map(|region| {
            let [x, y, w, h] = region.rect;
            [x as i32, y as i32, (x + w) as i32, (y + h) as i32]
        });
        for (place, (item, hover)) in rows.places.iter().zip(&laid) {
            let Some(place) = place else { continue };
            items[at].extend(holder.place(*place, item));
            if let Some(hover) = hover {
                // The row's own frame, placed as its parts are, within the list's clip.
                let k = holder.scale;
                let rect = [
                    holder.origin[0] + k * place[0] as f32,
                    holder.origin[1] + k * place[1] as f32,
                    k * item.size[0] as f32,
                    k * item.size[1] as f32,
                ];
                hovers.push((
                    rect,
                    intersect(holder.clip, list_box),
                    Some(path.clone()),
                    hover.clone(),
                ));
            }
        }
        items[at].extend(holder.place_scrollbar(&rows));
        if let Some(region) = region {
            let [x, y, w, h] = region.rect;
            scenes.push(SceneList {
                path: path.clone(),
                rect: [x as i32, y as i32, w as i32, h as i32],
                offset,
                max,
                horizontal,
                scale: holder.scale,
            });
        }
    }
    for bound in grids {
        let path = &bound.grid.path;
        let Some(at) = layout
            .holders
            .iter()
            .position(|holder| &holder.path == path)
        else {
            // A hidden grid box holds no items.
            continue;
        };
        let holder = &layout.holders[at];
        let places = native_gui::finalize_grid_positions(
            bound.grid,
            holder.content,
            instance.viewport(),
            bound.items.iter().map(|(_, slot)| (bound.template, *slot)),
            holder.position,
        )
        .map_err(|refusal| refused(vec![refusal]))?
        .places();
        for ((row, _), place) in bound.items.iter().zip(places) {
            let Some(item) = row
                .instance
                .layout_with_regions(bound.template, resources, None, true)
                .map_err(refused)?
            else {
                notes.note(format!(
                    "{name}: a {} item draws nothing",
                    bound.template.path
                ));
                continue;
            };
            for diagnostic in &item.diagnostics {
                notes.note(noted_cause(name, diagnostic));
            }
            items[at].extend(holder.place(place, &item));
            if let Some(hover) = &row.hover {
                let k = holder.scale;
                let rect = [
                    holder.origin[0] + k * place[0] as f32,
                    holder.origin[1] + k * place[1] as f32,
                    k * item.size[0] as f32,
                    k * item.size[1] as f32,
                ];
                // A row of a scrolling container: focusing it scrolls that container.
                let area = parent(path)
                    .filter(|parent| scrolled.iter().any(|(area, ..)| area == parent))
                    .map(str::to_owned);
                hovers.push((rect, holder.clip, area, hover.clone()));
            }
        }
    }
    for (path, vertical, [offset, max]) in scrolled {
        // Offset units are the content's pixels, at the scale of the grid it holds.
        let (Some(region), Some(holder)) = (
            layout.regions.iter().find(|region| region.path == path),
            layout
                .holders
                .iter()
                .find(|holder| parent(&holder.path) == Some(path.as_str())),
        ) else {
            continue;
        };
        let [x, y, w, h] = region.rect;
        scenes.push(SceneList {
            path,
            rect: [x as i32, y as i32, w as i32, h as i32],
            offset,
            max,
            horizontal: !vertical,
            scale: holder.scale,
        });
    }
    for (path, hover) in control_hovers {
        // A hidden control has no region and explains nothing.
        if let Some(region) = layout.regions.iter().find(|region| region.path == *path) {
            hovers.push((region.rect, region.clip, None, hover.clone()));
        }
    }
    let hovers: Vec<SceneHover> = hovers
        .into_iter()
        .map(|(rect, clip, list, hover)| SceneHover {
            rect,
            clip,
            list,
            title: hover.title,
            details: hover.details,
        })
        .collect();
    Ok(Laid {
        name: name.to_owned(),
        size: layout.size,
        viewport: instance.viewport(),
        parts: layout.composed_parts(&items).cloned().collect(),
        hovers,
        lists: scenes,
        notes: notes.lines(),
    })
}

/// A window laid out: every part in the game's draw order, the areas the app explains on hover,
/// the lists the pointer scrolls and what the layout noted.
#[derive(Debug)]
pub struct Laid {
    pub name: String,
    pub size: [i32; 2],
    pub viewport: [i32; 2],
    pub parts: Vec<Part>,
    pub hovers: Vec<SceneHover>,
    pub lists: Vec<SceneList>,
    pub notes: Vec<String>,
}

impl Laid {
    /// The window painted as the game draws it ([`crate::paint`]); a part the native draw
    /// refuses is left out, named, and the scene says it is incomplete.
    pub fn paint(self, canvas: Canvas<'_>) -> Result<Scene, Refused> {
        let name = &self.name;
        let painted = crate::paint::paint(
            canvas.painter,
            canvas.pages,
            canvas.sheets,
            &self.parts,
            self.size,
        )
        .map_err(|why| vec![format!("{name} is not painted: {why}")])?;
        // A scene is painted for one hour and never shown again: kept as artwork, it would stay for
        // the process's life.
        let image = dashboard_text::artwork::png_data_url(&painted.image)
            .map_err(|why| vec![format!("{name} is not painted: {why}")])?;
        let incomplete = !painted.refused.is_empty();
        let mut notes = self.notes;
        notes.extend(painted.refused.lines());
        if incomplete {
            notes.push(format!(
                "{name} is drawn incompletely: the parts named are left out"
            ));
        }
        Ok(Scene {
            name: self.name,
            size: self.size,
            viewport: self.viewport,
            image,
            hovers: self.hovers,
            lists: self.lists,
            notes,
        })
    }
}

/// The instance with each grid's items finalized into it and each grid's enclosing container
/// refreshed and scrolled to its offsets in `scroll`, and each shown bar of a container: its
/// path, axis, and the offset reached and the range's far end, both in pixels past the range's
/// near end.
#[allow(clippy::type_complexity)]
fn scroll_grids(
    window: &Window<'_>,
    scroll: &Scroll,
    resources: &dyn Resources,
    refused: &dyn Fn(Refusals) -> Refused,
) -> Result<(Instance, Vec<(String, bool, [i32; 2])>), Refused> {
    let root = window.root;
    let mut instance = window.instance.clone();
    let layout = instance
        .layout_with_regions(root, resources, None, true)
        .map_err(refused)?
        .ok_or_else(|| vec![format!("{} draws nothing", window.name)])?;
    let mut containers: Vec<&str> = Vec::new();
    for bound in window.grids {
        let path = &bound.grid.path;
        let Some(holder) = layout.holders.iter().find(|holder| &holder.path == path) else {
            continue;
        };
        let item = native_gui::container_frame(bound.template, Some(instance.viewport()))
            .map_err(|why| vec![format!("{}: {why}", window.name)])?;
        let placed = native_gui::finalize_grid_positions(
            bound.grid,
            holder.content,
            instance.viewport(),
            bound.items.iter().map(|(_, slot)| (bound.template, *slot)),
            holder.position,
        )
        .map_err(|refusal| refused(vec![refusal]))?;
        let rectangles: Vec<_> = placed.items.iter().map(|&at| (at, item.size)).collect();
        instance
            .finalize_grid_resolved(root, bound.grid, holder, &rectangles)
            .map_err(refused)?;
        if let Some(parent) = parent(path)
            && !containers.contains(&parent)
        {
            containers.push(parent);
        }
    }
    let mut scrolled = Vec::new();
    for parent in containers {
        let Ok(container) = instance.element_at_path(root, parent) else {
            continue;
        };
        let Some(fields) = container.container() else {
            continue;
        };
        instance
            .synchronize_scroll(root, container, resources, false)
            .map_err(refused)?;
        for (vertical, name) in [
            (true, &fields.vertical_scrollbar),
            (false, &fields.horizontal_scrollbar),
        ] {
            if installed_content_core::parser::c_string(name).is_empty() {
                continue;
            }
            // The scene's offsets run from the range's near end, which can lie below zero (a
            // grid growing up or left out of its container): an asked offset is past that end,
            // and absent asks for the content where construction leaves it.
            let asked = scroll.get(&(parent.to_owned(), !vertical)).copied();
            // Each shown bar scrolls its own axis.
            if let Some(([near, far], reached)) = instance
                .scroll_to(
                    root,
                    container,
                    vertical,
                    |[near, _]| asked.map(|asked| near.wrapping_add(asked)),
                    resources,
                )
                .map_err(refused)?
            {
                scrolled.push((
                    parent.to_owned(),
                    vertical,
                    [reached.wrapping_sub(near), far.wrapping_sub(near)],
                ));
            }
        }
    }
    Ok((instance, scrolled))
}

fn intersect(a: Option<[i32; 4]>, b: Option<[i32; 4]>) -> Option<[i32; 4]> {
    match (a, b) {
        (Some(a), Some(b)) => Some([
            a[0].max(b[0]),
            a[1].max(b[1]),
            a[2].min(b[2]),
            a[3].min(b[3]),
        ]),
        (one, None) | (None, one) => one,
    }
}

/// Constructs `template` as the game constructs a window its code shows.
pub fn construct(
    template: &Element,
    viewport: [i32; 2],
    resources: &dyn Resources,
) -> Result<Instance, String> {
    let mut instance = Instance::construct(template, viewport, resources)?;
    instance.gui_show(template, template)?;
    Ok(instance)
}

/// The rows' laid-out regions and parts are plain data; a binder that needs one control of a
/// row finds it by its path below the row's template.
pub fn control<'a>(template: &'a Element, path: &str) -> Result<&'a Element, String> {
    template
        .path_named(path)
        .ok_or_else(|| format!("{} has no {path} control", template.path))
}

pub use native_gui::SpriteSetter;
