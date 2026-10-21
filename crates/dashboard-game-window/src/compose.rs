//! Lays a bound window and the rows its code adds to its list boxes out through the native
//! GUI, as the map viewer lays its windows out, and flattens the result into a [`Scene`].
//!
//! A window's binder constructs the root [`Instance`] and one per row, applies the calls the
//! game's code makes (texts, sprites, frames, shows and hides), and hands them here with what
//! the app explains for each row on hover. Rows are placed where the list box places them at
//! the list's scroll offset ([`widgets::list_box_rows`]), its scrollbar after them, and every
//! part is drawn in the game's attach order ([`native_gui::Layout::composed_parts`]).

use std::collections::BTreeMap;

use installed_content_core::gui::native::{Element, Fields, Kind};
use viewer_foundation::map::{
    gui_resources::{noted_cause, refused_cause},
    native_gui::{self, Bindings, Part, Refusals, Resources, Sheets, lifecycle::Instance, widgets},
};

use crate::scene::{Counted, Flatten, Scene, SceneHover, SceneList};

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
    /// Drawn muted, as the app mutes a row that cannot be acted on ([`crate::scene::SceneQuad::muted`]).
    pub muted: bool,
}

/// A list box of the root and the rows the code adds to it, in order.
pub struct BoundList<'a> {
    pub list: &'a Element,
    pub template: &'a Element,
    pub rows: Vec<Row>,
}

/// Why a window is not drawn, each reason counted.
pub type Refused = Vec<String>;

/// Lays `root` out with its bound lists, each scrolled to its offset in `scroll` (clamped to
/// its range; zero when absent).
#[allow(clippy::too_many_arguments)]
pub fn compose(
    name: &str,
    root: &Element,
    instance: &Instance,
    lists: &[BoundList<'_>],
    scroll: &BTreeMap<String, i32>,
    resources: &dyn Resources,
    sheets: &dyn Sheets,
    sizes: &dyn Fn(usize) -> Option<[u32; 2]>,
) -> Result<Scene, Refused> {
    let refused = |refusals: Refusals| -> Refused {
        let mut counted = Counted::default();
        for refusal in &refusals {
            counted.note(refused_cause(name, refusal));
        }
        counted.lines()
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
    // The parts of muted rows, by their range in their holder's items.
    let mut muted: Vec<Vec<std::ops::Range<usize>>> = vec![Vec::new(); layout.holders.len()];
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
                    laid.push((item, row.hover.clone(), row.muted));
                }
                None => notes.note(format!(
                    "{name}: a {} row draws nothing",
                    bound.template.path
                )),
            }
        }
        let sizes: Vec<[i32; 2]> = laid.iter().map(|(item, ..)| item.size).collect();
        // A smooth list box scrolls by pixels over the rows' run beyond its own extent.
        let horizontal = list.horizontal != 0;
        let axis = usize::from(!horizontal);
        let run: i32 = sizes
            .iter()
            .map(|size| size[axis])
            .fold(0, i32::wrapping_add);
        let max = run.wrapping_sub(list.size[axis]).max(0);
        // A standard list box scrolls by whole rows, which the dashboard does not offer yet: it
        // shows its rows from the first, as the game first shows them.
        let smooth = bound.list.kind == Kind::SmoothListBox;
        let max = if smooth { max } else { 0 };
        let offset = scroll.get(path).copied().unwrap_or(0).clamp(0, max);
        let current = smooth.then_some(offset as f32);
        let rows = widgets::list_box_rows(bound.list, &sizes, current, resources, &bindings)
            .map_err(refused)?;
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
        for (place, (item, hover, dim)) in rows.places.iter().zip(&laid) {
            let Some(place) = place else { continue };
            let first = items[at].len();
            items[at].extend(holder.place(*place, item));
            if *dim {
                muted[at].push(first..items[at].len());
            }
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
                    path.clone(),
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
    let mut flatten = Flatten::new(sheets, sizes);
    let is_muted = |part: &Part| {
        items.iter().zip(&muted).any(|(items, ranges)| {
            ranges.iter().any(|range| {
                items[range.clone()]
                    .as_ptr_range()
                    .contains(&std::ptr::from_ref(part))
            })
        })
    };
    for part in layout.composed_parts(&items) {
        flatten.part(part, is_muted(part));
    }
    let hovers: Vec<SceneHover> = hovers
        .into_iter()
        .map(|(rect, clip, list, hover)| SceneHover {
            rect,
            clip: flatten.clip(clip),
            list: Some(list),
            title: hover.title,
            details: hover.details,
        })
        .collect();
    let (quads, textures, clips, effects, flattened, omitted) = flatten.finish();
    let mut notes = notes.lines();
    notes.extend(flattened.lines());
    if omitted > 0 {
        notes.push(format!(
            "{name} is drawn incompletely: {omitted} part(s) the dashboard cannot draw yet are left out"
        ));
    }
    Ok(Scene {
        name: name.to_owned(),
        size: layout.size,
        viewport: instance.viewport(),
        textures,
        clips,
        quads,
        hovers,
        lists: scenes,
        effects: effects.lines(),
        notes,
    })
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
