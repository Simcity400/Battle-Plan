//! Retained combat windows and code-added entries through the native GUI walk.
use super::{Drawn, ScrollTarget, Selection, TooltipHit, Values, binding::Bindings};
use installed_content_core::{crt::truncate_f32, gui::native::Element, gui::native::Kind};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};
use viewer_foundation::map::{
    gui_resources as gui_windows,
    native_gui::{
        self, Layout, Part, Pointer, Region, Resources, SpriteKind, SpriteSetter,
        lifecycle::{Instance, Tick},
        pointer::{self, Input, Played, PointerState, Step},
    },
    native_windows::NativeWindows,
};

/// Recorded flag keys resolved for this presentation, over the native draw resources.
pub trait CombatResources: Resources + native_gui::Sheets {
    fn flag_index(&self, key: &str) -> Result<i32, String>;
    fn flag_texture(&self, key: &str, small: bool) -> Option<native_gui::Texture>;
    /// The sounds `button` plays, from its sprite's declaration and the GUI's defaults.
    fn button_sounds(
        &self,
        button: &installed_content_core::gui::native::Button,
    ) -> installed_content_core::gui::native::ButtonSounds;
    fn line_chart(
        &self,
        _sprite: &[u8],
    ) -> Result<(viewer_foundation::map::native_shader::draw::Effect, f32), String> {
        Err("line chart has no compiled content effect".into())
    }
}

#[derive(Default)]
pub struct Windows {
    built: Option<(NativeWindows, [i32; 2])>,
    root: Option<Retained>,
    popup: Option<Retained>,
    pending: Vec<ScrollAction>,
    /// Where the tactics popup drew last frame: the pointer over it is not the window's.
    popup_rect: Option<[f32; 4]>,
    /// Where the window drew last frame.
    root_rect: Option<[f32; 4]>,
}

pub(crate) struct Retained {
    name: String,
    pub(crate) instance: Instance,
    entries: BTreeMap<String, Retained>,
    pending: Vec<ScrollAction>,
    prepared_generation: Option<u64>,
    places: BTreeMap<String, Vec<(usize, [i32; 2])>>,
    bind_refused: BTreeMap<String, usize>,
    bind_failed: bool,
    pointer: PointerState,
    /// Each placed item's window rectangle as last laid out, by key, in draw order.
    item_rects: Vec<(String, [f32; 4], usize, Option<native_gui::Clip>)>,
    /// Where each item was placed: its key, holder, place and its own root region, which a
    /// scroll before routing places again.
    item_places: Vec<(String, usize, [i32; 2], Region)>,
}

struct ScrollAction {
    target: ScrollTarget,
    direction: i32,
    wheel: bool,
}

pub(crate) struct Row<'a> {
    pub(crate) naval_layout: Option<super::binding::NavalLayout>,
    pub(crate) key: String,
    pub(crate) template: &'a str,
    pub(crate) values: &'a Values,
    /// Runtime size and optional explicit pixels; None pixels use authored grid slots.
    pub(crate) explicit: Option<(Option<[f32; 2]>, [f32; 2])>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GridPlacement {
    Authored,
    AutomaticNaval,
    ExplicitNaval,
}

impl Row<'_> {
    fn placement(&self) -> GridPlacement {
        match self.explicit {
            None => GridPlacement::Authored,
            Some((None, _)) => GridPlacement::AutomaticNaval,
            Some((Some(_), _)) => GridPlacement::ExplicitNaval,
        }
    }
}

pub(crate) struct Group<'a> {
    pub(crate) path: String,
    pub(crate) rows: Vec<Row<'a>>,
}

pub(crate) struct Prepared {
    pub(crate) layout: Layout,
    pub(crate) items: Vec<Vec<Part>>,
    pub(crate) hovers: Vec<Hover>,
    /// Each placed grid entry's root region in window coordinates: its group path and row key.
    pub(crate) entries: Vec<(String, String, Region)>,
    /// The sound effects the window's and its items' button state changes play this frame.
    pub(crate) sounds: Vec<Played>,
    /// The controls clicked this frame, in order: an item's by its row key, the window's own by
    /// `None`, with each click's [`pointer::Step::order`].
    pub(crate) clicks: Vec<(Option<String>, String, usize)>,
}

pub(crate) struct Hover {
    order: usize,
    region: Region,
    text: Option<super::HoverText>,
    description: Option<super::HoverText>,
    shortcut: Option<super::HoverText>,
    anchor: super::TooltipAnchor,
}

fn count(refused: &mut BTreeMap<String, usize>, path: &str, why: impl std::fmt::Display) {
    *refused
        .entry(format!("Combat GUI {path}: {why}"))
        .or_default() += 1;
}

fn naval_rectangles(
    grid: &Element,
    parent: [i32; 2],
    viewport: [i32; 2],
    entries: &[(&Element, [i32; 2])],
) -> Result<Vec<([i32; 2], [i32; 2])>, String> {
    let mut height = 0_i32;
    let slots = entries
        .iter()
        .map(|&(template, size)| {
            let frame = native_gui::container_frame(template, Some(viewport))?;
            let slot = [size[0].wrapping_sub(frame.size[0]) / 2, height];
            height = height.wrapping_add(size[1]).wrapping_add(frame.size[1]);
            Ok((template, slot))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let placed = native_gui::finalize_grid_positions(grid, parent, viewport, slots, None)
        .map_err(|why| why.to_string())?;
    Ok(placed
        .items
        .into_iter()
        .zip(entries.iter().map(|(_, size)| *size))
        .collect())
}

fn naval_label_offset(
    root: &Element,
    width: i32,
    layout: super::binding::NavalLayout,
) -> Option<(&Element, i32)> {
    use super::binding::NavalLayout;
    let right_control = root.path_named("desc_right")?;
    let right = right_control.text_box()?;
    let text_width = if right.max_width == 0 {
        200
    } else {
        right.max_width as i32
    };
    match layout {
        NavalLayout::AirLeft => {
            let left_control = root.path_named("desc_left")?;
            let left = left_control.text_box()?;
            Some((
                left_control,
                width
                    .wrapping_sub(right.position[0])
                    .wrapping_sub(left.position[0]),
            ))
        }
        NavalLayout::AirRight => {
            let left = root.path_named("desc_left")?.text_box()?;
            Some((
                right_control,
                left.position[0]
                    .wrapping_sub(text_width)
                    .wrapping_sub(right.position[0]),
            ))
        }
        NavalLayout::Ship | NavalLayout::Sunk => {
            Some((right_control, width.wrapping_sub(text_width)))
        }
    }
}

pub(crate) fn template<'a>(windows: &'a NativeWindows, name: &str) -> Result<&'a Element, String> {
    windows
        .gui_type(name.as_bytes())
        .ok_or_else(|| format!("the content stack declares no {name} template"))
}

pub(crate) fn ensure<'a>(
    kept: &'a mut Option<Retained>,
    name: &str,
    root: &Element,
    viewport: [i32; 2],
    resources: &impl CombatResources,
) -> Result<&'a mut Retained, String> {
    if kept.as_ref().is_none_or(|kept| kept.name != name) {
        *kept = Some(Retained::new(name, root, viewport, resources)?);
    }
    Ok(kept.as_mut().expect("constructed above"))
}

impl Retained {
    /// Whether the window draws differently next frame without a call from the code: an animation
    /// of its own, or a pointer state still settling.
    pub(crate) fn active(&self) -> bool {
        self.instance.active() || self.pointer.changing() || self.entries.values().any(Self::active)
    }

    /// The control the last draw drew for the pointer: an item's, drawn above the window, first.
    pub(crate) fn highlighted(&self) -> Option<&str> {
        self.entries
            .values()
            .find_map(Self::highlighted)
            .or_else(|| self.pointer.highlighted())
    }

    /// Whether the primary button holds a button of the window or of an item.
    pub(crate) fn holding(&self) -> bool {
        self.pointer.holding() || self.entries.values().any(Self::holding)
    }

    /// Ends one pointer step on `layout`: an item drawn over the window's hovered button takes
    /// the pointer instead (`covered`), and a press that took a button lays out again to draw
    /// Down. The layout to draw, and whether an item covered the window.
    fn settle(
        &mut self,
        root: &Element,
        resources: &impl CombatResources,
        refused: &mut BTreeMap<String, usize>,
        mut layout: Layout,
        item_under: Option<usize>,
    ) -> Option<(Layout, bool)> {
        // The item covers the topmost control under the pointer, check boxes included, drawn
        // before the item's own holder.
        let covered = item_under.is_some_and(|holder| {
            layout.target.as_ref().is_none_or(|target| {
                layout
                    .parts
                    .iter()
                    .rposition(|part| part.path == target.path)
                    .is_none_or(|at| layout.holders.get(holder).is_some_and(|h| at < h.before))
            })
        });
        let again = |state: &PointerState, refused: &mut BTreeMap<String, usize>| {
            laid(&self.instance, root, resources, refused, false, Some(state.pointer()))
        };
        if covered && layout.target.is_some() {
            self.pointer.withdraw();
            layout = again(&self.pointer, refused)?;
        }
        if self.pointer.end(&layout) {
            layout = again(&self.pointer, refused)?;
            self.pointer.end(&layout);
        }
        Some((layout, covered))
    }

    /// Whether a pointer state of the window or an item still changes on the clock.
    pub(crate) fn pointing(&self) -> bool {
        self.pointer.changing() || self.entries.values().any(Self::pointing)
    }

    fn new(
        name: &str,
        root: &Element,
        viewport: [i32; 2],
        resources: &impl CombatResources,
    ) -> Result<Self, String> {
        let mut instance = Instance::construct(root, viewport, resources)?;
        instance.gui_show(root, root)?;
        Ok(Self {
            name: name.to_owned(),
            instance,
            entries: BTreeMap::new(),
            pending: Vec::new(),
            prepared_generation: None,
            places: BTreeMap::new(),
            bind_refused: BTreeMap::new(),
            bind_failed: false,
            pointer: PointerState::default(),
            item_rects: Vec::new(),
            item_places: Vec::new(),
        })
    }

    fn bind(
        &mut self,
        root: &Element,
        values: &Values,
        size: Option<[f32; 2]>,
        naval_layout: Option<super::binding::NavalLayout>,
        resources: &impl CombatResources,
        refused: &mut BTreeMap<String, usize>,
    ) -> Option<[i32; 2]> {
        apply(&mut self.instance, root, values, resources, refused);
        let mut size = size?.map(truncate_f32);
        // Escaping entries grow by the bound escape bar's queried height.
        if ["escape_icon_left", "escape_icon_right"]
            .iter()
            .any(|key| values.get(*key).is_some_and(|widget| !widget.hidden))
        {
            if let Some(bar) = root.path_named("escape_progress") {
                match self.instance.drawable_extent(root, bar, resources) {
                    Ok(Some(extent)) => size[1] = size[1].wrapping_add(extent[1]),
                    Ok(None) => count(
                        refused,
                        &root.path,
                        "escaping entry has no escape-bar drawable extent",
                    ),
                    Err(why) => gui_windows::count_refused(refused, &root.path, &why),
                }
            } else {
                count(
                    refused,
                    &root.path,
                    "escaping entry has no native escape_progress control",
                );
            }
        }
        if let Err(why) = self.instance.set_size(root, root, size, resources) {
            gui_windows::count_refused(refused, &root.path, &why);
        }
        let constructed = native_gui::container_frame(root, Some(self.instance.viewport()));
        let runtime = self.instance.content_size(root, root);
        match (constructed, runtime) {
            (Ok(constructed), Ok(runtime)) => {
                if let Some(layout) = naval_layout
                    && let Some((control, offset)) = naval_label_offset(root, runtime[0], layout)
                    && let Ok(bound) = self.instance.control(root, control)
                {
                    bound.offset = Some([offset, 0]);
                }
                // The native updater explicitly centres these controls after its flag-0 resize.
                for key in [
                    "bar_str",
                    "bar_org",
                    "icon_hidden",
                    "pride",
                    "escape_progress",
                    "escape_icon_left",
                    "escape_icon_right",
                ] {
                    let Some(control) = root.path_named(key) else {
                        continue;
                    };
                    let Some(icon) = control.icon() else { continue };
                    let from =
                        native_gui::control_anchor(icon.orientation, constructed.content_size);
                    if let Some(naval_layout) = naval_layout {
                        if key == "pride" && naval_layout == super::binding::NavalLayout::Sunk {
                            continue;
                        }
                        if values.get(key).is_some_and(|widget| widget.hidden) {
                            continue;
                        }
                        let centered = if key.starts_with("escape_icon_") {
                            root.path_named("escape_progress").and_then(|bar| {
                                let icon = bar.icon()?;
                                self.instance
                                    .drawable_extent(root, bar, resources)
                                    .ok()
                                    .flatten()
                                    .map(|extent| {
                                        runtime[0] / 2 - extent[0] / 2
                                            + truncate_f32(icon.position[0])
                                    })
                            })
                        } else {
                            self.instance
                                .drawable_extent(root, control, resources)
                                .ok()
                                .flatten()
                                .map(|extent| runtime[0] / 2 - extent[0] / 2)
                        };
                        if let Some(x) = centered {
                            if let Ok(bound) = self.instance.control(root, control) {
                                bound.offset = Some([x.wrapping_sub(from[0]), 0]);
                            }
                        } else {
                            count(
                                refused,
                                &control.path,
                                "naval entry control has no drawable extent for centering",
                            );
                        }
                        continue;
                    }
                    let to = native_gui::control_anchor(icon.orientation, runtime);
                    match self.instance.control(root, control) {
                        Ok(bound) => {
                            let offset = bound.offset.unwrap_or([0; 2]);
                            bound.offset = Some([0, 1].map(|axis| {
                                offset[axis].wrapping_add(to[axis].wrapping_sub(from[axis]))
                            }));
                        }
                        Err(why) => count(refused, &control.path, why),
                    }
                }
            }
            (Err(why), _) | (_, Err(why)) => count(refused, &root.path, why),
        }
        Some(size)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare<'v>(
        &mut self,
        root: &Element,
        values: &'v Values,
        mut groups: Vec<Group<'v>>,
        windows: &NativeWindows,
        resources: &impl CombatResources,
        tick: Tick,
        generation: u64,
        advance: bool,
        hovers: bool,
        bind: bool,
        input: Option<Input>,
        refused: &mut BTreeMap<String, usize>,
    ) -> Option<Prepared> {
        groups.extend(repeated(values));
        if self.prepared_generation != Some(generation) {
            for (path, element) in elements(root) {
                if element.kind == Kind::Grid
                    && !groups.iter().any(|group| {
                        root.path_named(&group.path)
                            .is_some_and(|bound| std::ptr::eq(bound, element))
                    })
                {
                    groups.push(Group {
                        path,
                        rows: Vec::new(),
                    });
                }
            }
            let mut fresh_refused = BTreeMap::new();
            let refused = &mut fresh_refused;
            if bind {
                self.bind(root, values, None, None, resources, refused);
            }
            let mut places = BTreeMap::new();
            let mut used = BTreeSet::new();
            let first = laid(&self.instance, root, resources, refused, false, None);
            if let Some(first) = first.as_ref() {
                let mut synchronized = BTreeSet::new();
                for group in &groups {
                    let Some(grid) = root.path_named(&group.path) else {
                        count(
                            refused,
                            &group.path,
                            "bound holder is absent from the native window",
                        );
                        continue;
                    };
                    let Some(holder) = first.holders.iter().find(|holder| holder.path == grid.path)
                    else {
                        if !hidden_path(root, &group.path, values) {
                            count(
                                refused,
                                &group.path,
                                "bound holder has no native layout geometry",
                            );
                        }
                        continue;
                    };
                    let mut rows = Vec::new();
                    let mut dimensions = Vec::new();
                    let mut placement = None;
                    for (row_index, row) in group.rows.iter().enumerate() {
                        if grid.kind == Kind::Grid
                            && let Some(expected) = placement
                            && row.placement() != expected
                        {
                            count(
                                refused,
                                &grid.path,
                                format!(
                                    "mixed grid placements: expected {expected:?}, found {:?}; entry withheld",
                                    row.placement()
                                ),
                            );
                            continue;
                        }
                        match template(windows, row.template) {
                            Ok(template) => {
                                let key = format!("{}/{}", group.path, row.key);
                                if self
                                    .entries
                                    .get(&key)
                                    .is_none_or(|kept| kept.name != row.template)
                                {
                                    match Self::new(
                                        row.template,
                                        template,
                                        self.instance.viewport(),
                                        resources,
                                    ) {
                                        Ok(kept) => {
                                            self.entries.insert(key.clone(), kept);
                                        }
                                        Err(why) => {
                                            count(refused, &group.path, why);
                                            continue;
                                        }
                                    }
                                }
                                let kept = self.entries.get_mut(&key).expect("constructed above");
                                let size = kept.bind(
                                    template,
                                    row.values,
                                    row.explicit.map(|(_, size)| size),
                                    row.naval_layout,
                                    resources,
                                    refused,
                                );
                                placement.get_or_insert(row.placement());
                                dimensions.push(size);
                                rows.push((row_index, row, template));
                            }
                            Err(why) => count(refused, &group.path, why),
                        }
                    }
                    let positioned = if grid.kind == Kind::Grid {
                        let finalized = if placement == Some(GridPlacement::ExplicitNaval) {
                            let rectangles: Vec<_> = rows
                                .iter()
                                .zip(&dimensions)
                                .filter_map(|((_, row, _), size)| {
                                    let Some((Some(offset), raw_size)) = row.explicit else {
                                        count(refused, &grid.path, "explicit naval grid entry lacks combat-box pixels; entry withheld");
                                        return None;
                                    };
                                    Some((offset.map(truncate_f32), size.unwrap_or_else(|| raw_size.map(truncate_f32))))
                                })
                                .collect();
                            let rectangles =
                                if rows.iter().all(|(_, row, _)| row.naval_layout.is_some()) {
                                    let entries: Vec<_> = rows
                                        .iter()
                                        .zip(&rectangles)
                                        .map(|((_, _, template), (_, size))| (*template, *size))
                                        .collect();
                                    match naval_rectangles(
                                        grid,
                                        holder.content,
                                        self.instance.viewport(),
                                        &entries,
                                    ) {
                                        Ok(rectangles) => rectangles,
                                        Err(why) => {
                                            count(refused, &grid.path, why);
                                            continue;
                                        }
                                    }
                                } else {
                                    rectangles
                                };
                            self.instance
                                .finalize_grid_resolved(root, grid, holder, &rectangles)
                        } else {
                            let items: Vec<_> =
                                rows.iter().map(|(_, _, template)| *template).collect();
                            self.instance
                                .finalize_grid(root, grid, holder, &items)
                                .and_then(|finalized| {
                                    if placement != Some(GridPlacement::AutomaticNaval) {
                                        return Ok(finalized);
                                    }
                                    // the game uses automatic slots for sunk entries but
                                    // the game resizes each to its sprite before finalize.
                                    let rectangles: Vec<_> = finalized
                                        .items
                                        .iter()
                                        .zip(&dimensions)
                                        .zip(&rows)
                                        .filter_map(|((&at, size), (_, row, _))| {
                                            let Some((None, raw_size)) = row.explicit else {
                                                count(refused, &grid.path, "automatic naval grid entry lacks runtime size; entry withheld");
                                                return None;
                                            };
                                            Some((at, size.unwrap_or_else(|| raw_size.map(truncate_f32))))
                                        })
                                        .collect();
                                    self.instance.finalize_grid_resolved(
                                        root,
                                        grid,
                                        holder,
                                        &rectangles,
                                    )
                                })
                        };
                        match finalized {
                            Ok(finalized) => {
                                let generated = finalized.places();
                                rows.iter()
                                    .enumerate()
                                    .filter_map(|(index, (row_index, _, _))| {
                                        if let Some(place) = generated.get(index) {
                                            Some((*row_index, *place))
                                        } else {
                                            count(
                                                refused,
                                                &group.path,
                                                "entry exceeds the authored grid slots",
                                            );
                                            None
                                        }
                                    })
                                    .collect::<Vec<_>>()
                            }
                            Err(why) => {
                                gui_windows::count_refused(refused, &self.name, &why);
                                continue;
                            }
                        }
                    } else if grid.kind == Kind::List {
                        let size = match rows.first() {
                            Some((_, _, entry)) => {
                                match native_gui::container_frame(
                                    entry,
                                    Some(self.instance.viewport()),
                                ) {
                                    Ok(frame) => frame.size,
                                    Err(why) => {
                                        count(refused, &entry.path, why);
                                        continue;
                                    }
                                }
                            }
                            None => [0; 2],
                        };
                        match native_gui::list_places(
                            grid,
                            holder.content,
                            size,
                            rows.len(),
                            holder.position,
                        ) {
                            Ok(places) => places
                                .into_iter()
                                .filter_map(|(index, place)| {
                                    rows.get(index)
                                        .map(|(row_index, _, _)| (*row_index, place))
                                        .or_else(|| {
                                            count(
                                                refused,
                                                &group.path,
                                                "positioned entry has no bound row",
                                            );
                                            None
                                        })
                                })
                                .collect(),
                            Err(why) => {
                                count(refused, &group.path, why);
                                continue;
                            }
                        }
                    } else {
                        count(
                            refused,
                            &group.path,
                            format!(
                                "is a {}, not a native grid or overlapping list",
                                grid.kind.keyword()
                            ),
                        );
                        continue;
                    };
                    if grid.kind == Kind::Grid {
                        for (row_index, _) in &positioned {
                            let row = &group.rows[*row_index];
                            let key = format!("{}/{}", group.path, row.key);
                            let Some(kept) = self.entries.get_mut(&key) else {
                                count(refused, &group.path, "positioned entry has no bound row");
                                continue;
                            };
                            let Ok(entry) = template(windows, row.template) else {
                                count(
                                    refused,
                                    &group.path,
                                    "positioned entry has no native template",
                                );
                                continue;
                            };
                            if let Err(why) =
                                kept.instance
                                    .set_opacity(entry, entry, holder.opacity, true)
                            {
                                count(refused, &group.path, why);
                            }
                        }
                    }
                    used.extend(positioned.iter().map(|(row_index, _)| {
                        format!("{}/{}", group.path, group.rows[*row_index].key)
                    }));
                    places.insert(group.path.clone(), positioned);
                    if grid.kind == Kind::Grid {
                        synchronized.insert(
                            group
                                .path
                                .rsplit_once('/')
                                .map_or("", |(parent, _)| parent)
                                .to_owned(),
                        );
                    }
                }
                // Land refill finalizes, then synchronizes including hidden children.
                for parent in synchronized {
                    if let Some(container) = root.path_named(&parent)
                        && let Err(why) = self
                            .instance
                            .synchronize_scroll(root, container, resources, true)
                    {
                        gui_windows::count_refused(refused, &self.name, &why);
                    }
                }
            }
            self.entries.retain(|key, _| used.contains(key));
            self.bind_failed = first.is_none();
            self.places = places;
            self.bind_refused = fresh_refused;
            self.prepared_generation = Some(generation);
        }
        for (reason, times) in &self.bind_refused {
            *refused.entry(reason.clone()).or_default() += times;
        }
        if self.bind_failed {
            return None;
        }
        let scrolled = !self.pending.is_empty();
        for action in self.pending.drain(..) {
            let Some(container) = root.path_named(&action.target.path) else {
                count(
                    refused,
                    &action.target.path,
                    "scroll target is absent from the native window",
                );
                continue;
            };
            if container.text_box().is_some() {
                if let Err(why) = self.instance.scroll_text(
                    root,
                    container,
                    action.direction,
                    action.wheel,
                    resources,
                ) {
                    count(refused, &action.target.path, why);
                }
                continue;
            }
            let changed = if action.wheel {
                self.instance.wheel_scroll(
                    root,
                    container,
                    action.target.vertical,
                    action.direction,
                    resources,
                )
            } else {
                self.instance.step_scroll(
                    root,
                    container,
                    action.target.vertical,
                    action.direction,
                    resources,
                )
            };
            if let Err(why) = changed {
                gui_windows::count_refused(refused, &self.name, &why);
            }
        }
        // A scroll this frame moved the items: they route where it placed them.
        if scrolled
            && !self.item_places.is_empty()
            && let Some(fresh) = laid(&self.instance, root, resources, refused, false, None)
        {
            self.item_rects = self
                .item_places
                .iter()
                .filter_map(|(key, holder, place, region)| {
                    let placed = fresh
                        .holders
                        .get(*holder)?
                        .place_regions(*place, std::slice::from_ref(region))
                        .pop()?;
                    Some((key.clone(), placed.rect, *holder, placed.clip))
                })
                .collect();
        }
        // Each button change goes, where it happened, to the control holding the press until its
        // release; otherwise to the window, unless an item drawn over its control takes it.
        let mut holder = if self.pointer.holding() {
            Some(Owner::Window)
        } else {
            self.entries
                .iter()
                .find(|(_, entry)| entry.holding())
                .map(|(key, _)| Owner::Entry(key.clone()))
        };
        // The item each button change went to; `None` for the window.
        let mut routed: Vec<Option<String>> = Vec::new();
        let mut sounds = Vec::new();
        let mut pointer = None;
        let mut held_by_item = None;
        let mut item_under = None;
        if let Some(input) = &input {
            self.pointer.new_frame();
            for button in &input.buttons {
                let held_by = match &holder {
                    Some(Owner::Entry(key)) => Some(key.clone()),
                    _ => None,
                };
                let place = button.place.filter(|_| held_by.is_none());
                let item = place
                    .filter(|_| holder.is_none())
                    .and_then(|place| item_at(&self.item_rects, place));
                let item_holder = item.as_ref().map(|(_, holder)| *holder);
                let item = item.map(|(key, _)| key);
                let step = Step {
                    place,
                    press: button.down,
                    held: button.down,
                    order: button.order,
                };
                let at = self.pointer.begin(&mut self.instance, step);
                let layout = laid(&self.instance, root, resources, refused, false, Some(at))?;
                let (_, covered) = self.settle(root, resources, refused, layout, item_holder)?;
                let to = held_by.or(item.filter(|_| covered));
                holder = match &to {
                    Some(key) => button.down.then(|| Owner::Entry(key.clone())),
                    None => self.pointer.holding().then_some(Owner::Window),
                };
                routed.push(to);
            }
            // The last place: the held control's alone, else the window's or the item's over it.
            held_by_item = match &holder {
                Some(Owner::Entry(key)) if input.held => Some(key.clone()),
                _ => None,
            };
            let window_holds = holder == Some(Owner::Window) && input.held;
            let place = input.place.filter(|_| held_by_item.is_none());
            item_under = place
                .filter(|_| !window_holds)
                .and_then(|place| item_at(&self.item_rects, place));
            let step = Step {
                place,
                press: false,
                held: input.held,
                order: pointer::LAST,
            };
            pointer = Some(self.pointer.begin(&mut self.instance, step));
        }
        // Input precedes one smoothing tick in input processing.
        if advance && let Err(why) = self.instance.tick(root, resources, tick) {
            count(refused, &self.name, why);
            return None;
        }
        let mut layout = laid(&self.instance, root, resources, refused, true, pointer)?;
        let mut covered = false;
        if input.is_some() {
            let (settled, under) =
                self.settle(
                    root,
                    resources,
                    refused,
                    layout,
                    item_under.as_ref().map(|(_, holder)| *holder),
                )?;
            layout = settled;
            covered = under;
            sounds.extend(button_sounds(root, &self.pointer, resources));
        }
        // The item the last place reaches.
        let place_to =
            held_by_item.or_else(|| item_under.map(|(key, _)| key).filter(|_| covered));
        let mut clicks: Vec<(Option<String>, String, usize)> = self
            .pointer
            .clicks()
            .filter(|_| input.is_some())
            .map(|(path, order)| (None, path.to_owned(), order))
            .collect();
        let mut item_rects = Vec::new();
        let mut item_places = Vec::new();
        let mut hovered = if hovers {
            hover_regions(root, &layout.regions, values)
        } else {
            Vec::new()
        };
        let mut items = vec![Vec::new(); layout.holders.len()];
        let mut entries = Vec::new();
        for (holder_index, holder) in layout.holders.iter().enumerate() {
            let Some(group) = groups.iter().find(|group| {
                root.path_named(&group.path)
                    .is_some_and(|grid| grid.path == holder.path)
            }) else {
                continue;
            };
            let Some(positioned) = self.places.get(&group.path) else {
                continue;
            };
            let hover_order = if hovers && !positioned.is_empty() {
                let order = layout
                    .regions
                    .iter()
                    .position(|region| region.path == holder.path);
                if order.is_none() {
                    count(refused, &holder.path, "entry hover has no holder region");
                }
                order
            } else {
                None
            };
            for (row_index, place) in positioned {
                let Some(row) = group.rows.get(*row_index) else {
                    count(refused, &group.path, "positioned entry has no bound row");
                    continue;
                };
                let entry = match template(windows, row.template) {
                    Ok(entry) => entry,
                    Err(why) => {
                        count(refused, &group.path, why);
                        continue;
                    }
                };
                let key = format!("{}/{}", group.path, row.key);
                let Some(kept) = self.entries.get_mut(&key) else {
                    count(
                        refused,
                        &group.path,
                        "positioned entry has no retained native instance",
                    );
                    continue;
                };
                let Some(prepared) = kept.prepare(
                    entry,
                    row.values,
                    Vec::new(),
                    windows,
                    resources,
                    tick,
                    generation,
                    advance,
                    hovers,
                    false,
                    input.as_ref().map(|input| {
                        let mut item = input.item(holder, *place);
                        if place_to.as_ref() != Some(&key) {
                            item.place = None;
                        }
                        // Every change reaches the item in order; one it did not take, from away.
                        for (button, to) in item.buttons.iter_mut().zip(&routed) {
                            if to.as_ref() != Some(&key) {
                                button.place = None;
                            }
                        }
                        item
                    }),
                    refused,
                ) else {
                    continue;
                };
                sounds.extend(prepared.sounds.iter().map(|sound| sound.within(&key)));
                clicks.extend(
                    prepared
                        .clicks
                        .iter()
                        .map(|(_, path, order)| (Some(row.key.clone()), path.clone(), *order)),
                );
                if let Some(region) = prepared
                    .layout
                    .regions
                    .iter()
                    .find(|region| region.path == entry.path)
                    && let Some(placed) = holder
                        .place_regions(*place, std::slice::from_ref(region))
                        .pop()
                {
                    item_rects.push((key.clone(), placed.rect, holder_index, placed.clip));
                    item_places.push((key.clone(), holder_index, *place, region.clone()));
                    entries.push((group.path.clone(), row.key.clone(), placed));
                }
                items[holder_index].extend(
                    holder.place_parts(*place, prepared.layout.composed_parts(&prepared.items)),
                );
                if let Some(order) = hover_order {
                    // Native holder placement also moves nested hovers, without comparing their text.
                    let regions: Vec<_> = prepared
                        .hovers
                        .iter()
                        .map(|hit| hit.region.clone())
                        .collect();
                    hovered.extend(
                        holder
                            .place_regions(*place, &regions)
                            .into_iter()
                            .zip(prepared.hovers)
                            .map(|(region, hit)| Hover {
                                order,
                                region,
                                ..hit
                            }),
                    );
                }
            }
        }
        // Collision targets follow graphical attach order, including grid items
        // before later siblings.
        hovered.sort_by_key(|hit| hit.order);
        self.item_rects = item_rects;
        self.item_places = item_places;
        // The window's and its items' changes, as the button changes came.
        sounds.sort_by_key(|sound| sound.order);
        clicks.sort_by_key(|(_, _, order)| *order);
        Some(Prepared {
            layout,
            items,
            hovers: hovered,
            entries,
            sounds,
            clicks,
        })
    }
}

/// Who takes a pointer step: the window itself or one of its items.
#[derive(Clone, PartialEq)]
enum Owner {
    Window,
    Entry(String),
}

/// The topmost item drawn at `place`, within its rectangle and its clip, by key and holder.
fn item_at(
    rects: &[(String, [f32; 4], usize, Option<native_gui::Clip>)],
    place: pointer::Place,
) -> Option<(String, usize)> {
    rects
        .iter()
        .rev()
        .find(|(_, rect, _, clip)| {
            gui_windows::contains(*rect, place.at)
                && clip.is_none_or(|clip| {
                    (clip[0] as f32..clip[2] as f32).contains(&place.clip[0])
                        && (clip[1] as f32..clip[3] as f32).contains(&place.clip[1])
                })
        })
        .map(|(key, _, holder, _)| (key.clone(), *holder))
}

/// Each of the window's own controls clicked: its path, the rectangle it drew and the order.
fn clicked_at(
    prepared: &Prepared,
    origin: [i32; 2],
    clicks: &mut Vec<(String, [f32; 4], usize)>,
) {
    for (_, path, order) in prepared.clicks.iter().filter(|(key, _, _)| key.is_none()) {
        if let Some(rect) = prepared
            .layout
            .regions
            .iter()
            .rev()
            .find(|region| region.path == *path)
            .and_then(|region| region_rect(region, origin))
        {
            clicks.push((path.clone(), rect, *order));
        }
    }
}

/// The sound effects `pointer`'s state changes this frame play, by each button of `root`.
fn button_sounds(
    root: &Element,
    pointer: &PointerState,
    resources: &impl CombatResources,
) -> Vec<Played> {
    pointer
        .sounds()
        .iter()
        .filter_map(|(path, sound, order)| {
            let button = element_at(root, path)?.button()?;
            sound.played(path, &resources.button_sounds(button), *order)
        })
        .collect()
}

/// `root`'s element at `path`, nested ones included.
fn element_at<'a>(root: &'a Element, path: &str) -> Option<&'a Element> {
    let mut stack = vec![root];
    std::iter::from_fn(|| {
        let element = stack.pop()?;
        stack.extend(element.nested());
        Some(element)
    })
    .find(|element| element.path == path)
}

/// The click sound of `root`'s button at `path`, with that path, which keys its retained click.
pub(crate) fn click_sound(
    root: &Element,
    path: &str,
    resources: &impl CombatResources,
) -> Option<(String, Vec<u8>)> {
    let button = element_at(root, path)?.button()?;
    Some((path.to_owned(), resources.button_sounds(button).click?))
}

fn laid(
    instance: &Instance,
    root: &Element,
    resources: &impl CombatResources,
    refused: &mut BTreeMap<String, usize>,
    noted: bool,
    pointer: Option<Pointer>,
) -> Option<Layout> {
    match instance.layout_with_regions(root, resources, pointer, true) {
        Ok(Some(layout)) => {
            if noted {
                gui_windows::count_noted(refused, &root.path, &layout.diagnostics);
            }
            Some(layout)
        }
        Ok(None) => None,
        Err(why) => {
            gui_windows::count_refused(refused, &root.path, &why);
            None
        }
    }
}

/// the game selects each assigned-trait entry's template.
pub(super) fn entry_template<'a>(
    widget: &'a super::Widget,
    index: usize,
    default: &'a str,
) -> &'a str {
    widget
        .entry_templates
        .get(&index)
        .map_or(default, String::as_str)
}

fn repeated(values: &Values) -> Vec<Group<'_>> {
    values
        .iter()
        .filter_map(|(path, widget)| {
            let name = widget.entry_template.as_deref()?;
            Some(Group {
                path: path.clone(),
                rows: widget
                    .entries
                    .iter()
                    .enumerate()
                    .map(|(index, values)| Row {
                        naval_layout: None,
                        key: index.to_string(),
                        template: entry_template(widget, index, name),
                        values,
                        explicit: None,
                    })
                    .collect(),
            })
        })
        .collect()
}

pub(super) fn elements(root: &Element) -> Vec<(String, &Element)> {
    let mut out = Vec::new();
    let mut pending = vec![(String::new(), root)];
    while let Some((path, element)) = pending.pop() {
        for child in element.nested().into_iter().rev() {
            let key = if path.is_empty() {
                child.name().into_owned()
            } else {
                format!("{path}/{}", child.name())
            };
            pending.push((key, child));
        }
        out.push((path, element));
    }
    out
}

fn hidden_path(root: &Element, path: &str, values: &Values) -> bool {
    let values = bound_values(root, values);
    let mut key = String::new();
    for part in path.split('/') {
        if !key.is_empty() {
            key.push('/');
        }
        key.push_str(part);
        if root.path_named(&key).is_some_and(|element| {
            values
                .get(element.path.as_str())
                .map_or(element.hide, |widget| widget.hidden)
        }) {
            return true;
        }
    }
    false
}

// Native name lookup folds ASCII case (ASCII case folding); Values keep their producer keys.
fn bound_values<'r, 'v>(
    root: &'r Element,
    values: &'v Values,
) -> BTreeMap<&'r str, &'v super::Widget> {
    values
        .iter()
        .filter_map(|(path, widget)| {
            root.path_named(path)
                .map(|element| (element.path.as_str(), widget))
        })
        .collect()
}

fn apply(
    instance: &mut Instance,
    root: &Element,
    values: &Values,
    resources: &impl CombatResources,
    refused: &mut BTreeMap<String, usize>,
) {
    let indexed = bound_values(root, values);
    for (path, element) in elements(root) {
        if path.is_empty() {
            continue;
        }
        let widget = indexed.get(element.path.as_str()).copied();
        if element.text_box().is_some()
            || element
                .button()
                .is_some_and(|button| !button.font.is_empty())
            || widget.is_some_and(|widget| widget.text.is_some())
        {
            // Window updates own dynamic text; absent values never expose authored examples.
            if let Ok(bound) = instance.control(root, element) {
                bound.resolved_flags = widget.is_some_and(|widget| widget.text.is_some());
            }
            if let Err(why) = instance.set_text(
                root,
                element,
                widget
                    .and_then(|widget| widget.text.as_deref())
                    .unwrap_or("—"),
                resources,
            ) {
                count(refused, &path, why);
            }
            if widget.is_some_and(|widget| widget.text_scroll_lines == Some(true)) {
                let lines = widget
                    .and_then(|widget| widget.text.as_deref())
                    .map_or(0, |text| text.bytes().filter(|byte| *byte == b'\n').count());
                if let Err(why) = instance.set_text_scroll_lines(root, element, lines, resources) {
                    count(refused, &path, why);
                }
            }
        }
        let authored = element
            .icon()
            .map(|icon| icon.sprite.as_slice())
            .or_else(|| element.button().map(|button| button.sprite.as_slice()));
        let sprite = widget
            .and_then(|widget| widget.sprite.as_deref())
            .map(str::as_bytes)
            .or(authored);
        // the game calls drawable replacement only
        // after the tactic picture path exists. A native no-op keeps its clocks too.
        if !widget.is_some_and(|widget| widget.retain_sprite)
            && let Some(sprite) = sprite
        {
            let setter = if element.kind == Kind::Icon {
                SpriteSetter::Icon
            } else {
                SpriteSetter::ButtonStandard
            };
            if let Err(why) = instance.set_sprite(root, element, sprite, setter) {
                count(refused, &path, why);
            }
        }
        let Some(widget) = widget else {
            // Removing a bound field restores its authored presentation instead of retaining an old hour.
            if let Err(why) = instance.set_hide_flag(root, element, element.hide) {
                count(refused, &path, why);
            }
            let shown = if element.hide {
                instance.hide(root, element)
            } else {
                instance.gui_show(root, element)
            };
            if let Err(why) = shown {
                count(refused, &path, why);
            }
            if let Ok(bound) = instance.control(root, element) {
                bound.progress = None;
                bound.frame = None;
                bound.offset = None;
                bound.rotation = None;
                bound.color = None;
                bound.shield = None;
                bound.texture = None;
                bound.enabled = None;
            }
            continue;
        };
        if let Err(why) = instance.set_hide_flag(root, element, widget.hidden) {
            count(refused, &path, why);
        }
        let shown = if widget.hidden {
            instance.hide(root, element)
        } else {
            instance.gui_show(root, element)
        };
        if let Err(why) = shown {
            count(refused, &path, why);
        }
        if let Some(opacity) = widget.opacity
            && element.container().is_some()
            && let Err(why) = instance.set_opacity(root, element, opacity, true)
        {
            count(refused, &path, why);
        }
        if widget.frame.is_none()
            && let Ok(bound) = instance.control(root, element)
        {
            bound.frame = None;
        }
        if let Some(frame) = widget.frame
            && let Err(why) = instance.set_frame(root, element, frame.wrapping_add(1) as i32)
        {
            count(refused, &path, why);
        }
        let info = sprite.and_then(|name| resources.sprite(name));
        let progress = match (widget.progress, info.as_ref().map(|info| &info.kind)) {
            (Some(progress), Some(SpriteKind::Progress { .. })) => Some(progress),
            (Some(_), _) => {
                count(
                    refused,
                    &path,
                    "bound integer progress has no native progress-bar drawable",
                );
                None
            }
            (None, _) => match (widget.fill, info.as_ref().map(|info| &info.kind)) {
                (Some(fill), Some(SpriteKind::Progress { steps, .. })) => {
                    Some(truncate_f32(fill * *steps as f32))
                }
                (Some(_), _) => {
                    count(
                        refused,
                        &path,
                        "bound progress value has no native progress-bar drawable",
                    );
                    None
                }
                _ => None,
            },
        };
        let flag = widget.flag.as_deref().map(|key| {
            if matches!(
                info.as_ref().map(|info| &info.kind),
                Some(SpriteKind::MaskedShield { .. })
            ) {
                resources.flag_index(key).map(|index| (Some(index), None))
            } else {
                resources
                    .flag_texture(key, true)
                    .map(|mut texture| {
                        if let Some(size) = widget.size {
                            texture.size = size.map(truncate_f32);
                        }
                        (None, Some(texture))
                    })
                    .ok_or_else(|| format!("resolved flag atlas key {key} is unavailable"))
            }
        });
        let (shield, texture) = match flag {
            Some(Ok(binding)) => binding,
            Some(Err(why)) => {
                count(refused, &path, why);
                if let Err(why) = instance.hide(root, element) {
                    count(refused, &path, why);
                }
                (None, None)
            }
            None => (None, None),
        };
        match instance.control(root, element) {
            Ok(bound) => {
                bound.progress = progress;
                bound.offset = Some(widget.offset.map(truncate_f32));
                bound.rotation = widget.rotation;
                bound.color = widget
                    .color
                    .or_else(|| widget.opacity.map(|opacity| [1.0, 1.0, 1.0, opacity]));
                bound.shield = shield;
                bound.texture = texture;
                bound.enabled = widget.enabled;
            }
            Err(why) => count(refused, &path, why),
        }
        if widget.size.is_some() && widget.flag.is_none() && element.container().is_none() {
            count(
                refused,
                &path,
                "runtime drawable size has no native setter binding",
            );
        }
    }
    for path in values.keys().filter(|path| root.path_named(path).is_none()) {
        count(
            refused,
            path,
            "bound element is absent from the native window",
        );
    }
}

pub(crate) fn region_rect(region: &Region, origin: [i32; 2]) -> Option<[f32; 4]> {
    let [mut x, mut y, w, h] = region.rect;
    let mut right = x + w;
    let mut bottom = y + h;
    if let Some(clip) = region.clip {
        x = x.max(clip[0] as f32);
        y = y.max(clip[1] as f32);
        right = right.min(clip[2] as f32);
        bottom = bottom.min(clip[3] as f32);
    }
    (right > x && bottom > y).then_some([
        x + origin[0] as f32,
        y + origin[1] as f32,
        right - x,
        bottom - y,
    ])
}

// GUI reloading binds these native templates.
pub(super) fn window_name(selection: Selection) -> &'static str {
    match selection {
        Selection::Land(_) => "landcombatview",
        Selection::Naval(_) => "navalcombatview",
        Selection::NavalResult(_) => "navalcombatresultswindow",
        Selection::AirRegion(_) => "strategicairview",
    }
}

fn hover_regions(root: &Element, regions: &[Region], values: &Values) -> Vec<Hover> {
    let indexed = bound_values(root, values);
    // Grid items forward their provider through the entry container.
    // Children with their own handler keep it.
    let provider = values.get("").filter(|widget| widget.hover_owned);
    // instant-text hit testing rejects the constructor's
    // always-transparent bit, even with a handler.
    let mut transparent = BTreeSet::new();
    let mut pending = vec![root];
    while let Some(element) = pending.pop() {
        if element
            .text_box()
            .is_some_and(|text| text.always_transparent)
        {
            transparent.insert(element.path.as_str());
        }
        pending.extend(element.nested());
    }
    regions
        .iter()
        .enumerate()
        .filter_map(|(order, region)| {
            if transparent.contains(region.path.as_str()) {
                return None;
            }
            let widget = indexed
                .get(region.path.as_str())
                .copied()
                .filter(|widget| {
                    widget.hover_owned
                        || widget.tooltip.is_some()
                        || widget.tooltip_description.is_some()
                        || widget.tooltip_shortcut.is_some()
                })
                .or(provider)?;
            if widget.tooltip.is_none()
                && widget.tooltip_description.is_none()
                && widget.tooltip_shortcut.is_none()
                && !(provider.is_some() && widget.hover_owned)
            {
                return None;
            }
            Some(Hover {
                order,
                region: region.clone(),
                text: widget.tooltip.clone(),
                description: widget.tooltip_description.clone(),
                shortcut: widget.tooltip_shortcut.clone(),
                anchor: widget.tooltip_anchor,
            })
        })
        .collect()
}

impl Windows {
    pub(super) fn scroll(&mut self, target: ScrollTarget, direction: i32, wheel: bool) {
        self.pending.push(ScrollAction {
            target,
            direction,
            wheel,
        });
    }
    pub(super) fn active(&self) -> bool {
        self.root.as_ref().is_some_and(Retained::active)
            || self.popup.as_ref().is_some_and(Retained::active)
    }

    /// Whether the primary button holds a button of the window or its popup.
    pub(super) fn holding(&self) -> bool {
        self.root.as_ref().is_some_and(Retained::holding)
            || self.popup.as_ref().is_some_and(Retained::holding)
    }

    /// The control the last draw drew for the pointer: the popup's, drawn above, first.
    pub(super) fn highlighted(&self) -> Option<&str> {
        self.popup
            .as_ref()
            .and_then(Retained::highlighted)
            .or_else(|| self.root.as_ref().and_then(Retained::highlighted))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        windows: &NativeWindows,
        resources: &impl CombatResources,
        bindings: &Bindings,
        generation: u64,
        selection: Selection,
        offset: [f32; 2],
        tactics_open: bool,
        tactics_position: &mut Option<[f32; 2]>,
        extent: [f32; 2],
        hovers: bool,
        tick: Tick,
        advance: bool,
        input: Option<Input>,
    ) -> io::Result<Drawn> {
        let viewport = extent.map(truncate_f32);
        if self
            .built
            .as_ref()
            .is_none_or(|(built, size)| *size != viewport || !built.same(windows))
        {
            let pending = std::mem::take(&mut self.pending);
            *self = Self {
                built: Some((windows.clone(), viewport)),
                pending,
                ..Self::default()
            };
        }
        let name = window_name(selection);
        let mut drawn = Drawn::default();
        let popup_rect = self.popup_rect.take();
        let root_rect = self.root_rect.take();
        // A press the window or its popup holds, or lands on, keeps the pointer from the other.
        let holder = if self.popup.as_ref().is_some_and(Retained::holding) {
            Some(true)
        } else {
            self.root.as_ref().is_some_and(Retained::holding).then_some(false)
        };
        let owner = |at: [f32; 2]| {
            if popup_rect.is_some_and(|rect| gui_windows::contains(rect, at)) {
                Some(true)
            } else {
                root_rect
                    .is_some_and(|rect| gui_windows::contains(rect, at))
                    .then_some(false)
            }
        };
        let popup_input = input
            .as_ref()
            .map(|input| input.shared(holder, owner, true));
        let window_input = input.as_ref().map(|input| {
            input
                .shared(holder, owner, false)
                .filter(|at| popup_rect.is_none_or(|rect| !gui_windows::contains(rect, at)))
        });
        let root = match template(windows, name) {
            Ok(root) => root,
            Err(why) => {
                count(&mut drawn.refused, name, why);
                return Ok(drawn);
            }
        };
        let retained = match ensure(&mut self.root, name, root, viewport, resources) {
            Ok(kept) => kept,
            Err(why) => {
                count(&mut drawn.refused, name, why);
                return Ok(drawn);
            }
        };
        let (actions, popup_actions): (Vec<_>, Vec<_>) = self
            .pending
            .drain(..)
            .partition(|action| !action.target.popup);
        retained.pending.extend(actions);
        self.pending = popup_actions;
        let frame = native_gui::container_frame(root, Some(viewport)).map_err(super::invalid)?;
        let origin = [
            frame.at[0].wrapping_add(truncate_f32(offset[0])),
            frame.at[1].wrapping_add(truncate_f32(offset[1])),
        ];
        retained
            .instance
            .set_position(root, root, origin)
            .map_err(super::invalid)?;
        let mut groups = Vec::new();
        if matches!(selection, Selection::Land(_)) {
            // combat selection and list refill preserve producer membership order.
            for side in 0..4 {
                let path = format!(
                    "{}_window/{}_window/list",
                    if side % 2 == 0 {
                        "attacker"
                    } else {
                        "defender"
                    },
                    if side < 2 { "committed" } else { "reserves" }
                );
                let template = if side >= 2 {
                    "reserve_combat_entry"
                } else if side == 0 {
                    "division_combat_attacker_entry"
                } else {
                    "division_combat_defender_entry"
                };
                groups.push(Group {
                    path,
                    rows: bindings.entries[side]
                        .iter()
                        .map(|(unit, values)| Row {
                            naval_layout: None,
                            key: unit.to_string(),
                            template,
                            values,
                            explicit: None,
                        })
                        .collect(),
                });
            }
        }
        for (index, row) in bindings.naval_entries.iter().enumerate() {
            let at = groups
                .iter()
                .position(|group| group.path == row.grid)
                .unwrap_or_else(|| {
                    groups.push(Group {
                        path: row.grid.clone(),
                        rows: Vec::new(),
                    });
                    groups.len() - 1
                });
            groups[at].rows.push(naval_row(index, row));
        }
        if let Some(prepared) = retained.prepare(
            root,
            &bindings.values,
            groups,
            windows,
            resources,
            tick,
            generation,
            advance,
            hovers,
            true,
            window_input
                .as_ref()
                .map(|input| input.local(origin.map(|value| value as f32))),
            &mut drawn.refused,
        ) {
            drawn.sounds.extend(prepared.sounds.iter().cloned());
            clicked_at(&prepared, origin, &mut drawn.clicks);
            drawn.rect = [
                origin[0] as f32,
                origin[1] as f32,
                prepared.layout.size[0] as f32,
                prepared.layout.size[1] as f32,
            ];
            self.root_rect = Some(drawn.rect);
            if draw_prepared(
                &prepared.layout,
                &prepared.items,
                origin,
                resources,
                &mut drawn,
            ) {
                controls(root, &prepared.layout.regions, origin, &mut drawn, false);
                if let Some(chart) = &bindings.air_chart {
                    super::air_menu::history::draw(
                        root,
                        &prepared.layout,
                        origin,
                        resources,
                        chart,
                        &mut drawn,
                    );
                }
                if hovers {
                    append_hovers(&mut drawn.tooltips, prepared.hovers, origin);
                }
            }
        }
        if tactics_open && matches!(selection, Selection::Land(_)) {
            let popup_name = "tactics_list_view";
            let popup = match template(windows, popup_name) {
                Ok(popup) => popup,
                Err(why) => {
                    count(&mut drawn.refused, popup_name, why);
                    return Ok(drawn);
                }
            };
            let retained = match ensure(&mut self.popup, popup_name, popup, viewport, resources) {
                Ok(kept) => kept,
                Err(why) => {
                    count(&mut drawn.refused, popup_name, why);
                    return Ok(drawn);
                }
            };
            retained.pending.append(&mut self.pending);
            // The possible-tactics popup sits ten pixels right of the combat window.
            let position = *tactics_position
                .get_or_insert([drawn.rect[0] + drawn.rect[2] + 10.0, drawn.rect[1]]);
            let origin = position.map(truncate_f32);
            retained
                .instance
                .set_position(popup, popup, origin)
                .map_err(super::invalid)?;
            let groups = vec![Group {
                path: "tactics_list_container/tactics_list_gridbox".into(),
                rows: bindings
                    .tactics
                    .entries
                    .iter()
                    .enumerate()
                    .map(|(index, values)| Row {
                        naval_layout: None,
                        key: index.to_string(),
                        template: "tactics_list_entry",
                        values,
                        explicit: None,
                    })
                    .collect(),
            }];
            if let Some(prepared) = retained.prepare(
                popup,
                &bindings.tactics.values,
                groups,
                windows,
                resources,
                tick,
                generation,
                advance,
                hovers,
                true,
                popup_input
                    .as_ref()
                    .map(|input| input.local(origin.map(|value| value as f32))),
                &mut drawn.refused,
            ) {
                drawn.sounds.extend(prepared.sounds.iter().cloned());
                clicked_at(&prepared, origin, &mut drawn.clicks);
                let rect = [
                    origin[0] as f32,
                    origin[1] as f32,
                    prepared.layout.size[0] as f32,
                    prepared.layout.size[1] as f32,
                ];
                if draw_prepared(
                    &prepared.layout,
                    &prepared.items,
                    origin,
                    resources,
                    &mut drawn,
                ) {
                    drawn.tactics_rect = Some(rect);
                    self.popup_rect = Some(rect);
                    controls(popup, &prepared.layout.regions, origin, &mut drawn, true);
                    if hovers {
                        drawn.tooltips.push(TooltipHit {
                            rect,
                            text: None,
                            description: None,
                            shortcut: None,
                            anchor: super::TooltipAnchor::Pointer,
                        });
                        append_hovers(&mut drawn.tooltips, prepared.hovers, origin);
                    }
                }
            }
        } else {
            self.popup = None;
            for action in self.pending.drain(..) {
                count(
                    &mut drawn.refused,
                    &action.target.path,
                    "scroll target's tactics popup is closed",
                );
            }
        }
        Ok(drawn)
    }
}

fn naval_row(index: usize, row: &super::binding::NavalEntry) -> Row<'_> {
    Row {
        naval_layout: Some(row.layout),
        key: index.to_string(),
        // Sunk and live ship icons share this template.
        template: "navalcombat_ship_entry",
        values: &row.values,
        explicit: Some((row.offset, row.size)),
    }
}

pub(crate) fn draw_prepared(
    layout: &Layout,
    items: &[Vec<Part>],
    origin: [i32; 2],
    resources: &impl CombatResources,
    drawn: &mut Drawn,
) -> bool {
    let first = drawn.batch.quads.len();
    if let Err(why) = native_gui::draw_composed(layout, items, origin, resources, &mut drawn.batch)
    {
        gui_windows::count_refused(&mut drawn.refused, "combat window", &[why]);
        return false;
    }
    for (index, quad) in drawn.batch.quads.iter_mut().enumerate().skip(first) {
        quad.layer = super::window_layer(index);
    }
    true
}

pub(crate) fn append_hovers(target: &mut Vec<TooltipHit>, hovers: Vec<Hover>, origin: [i32; 2]) {
    target.extend(hovers.into_iter().filter_map(|hit| {
        Some(TooltipHit {
            rect: region_rect(&hit.region, origin)?,
            text: hit.text,
            description: hit.description,
            shortcut: hit.shortcut,
            anchor: hit.anchor,
        })
    }));
}

fn controls(root: &Element, regions: &[Region], origin: [i32; 2], drawn: &mut Drawn, popup: bool) {
    let indexed: BTreeMap<_, _> = elements(root)
        .into_iter()
        .map(|(path, element)| (element.path.as_str(), path))
        .collect();
    for region in regions {
        let Some(rect) = region_rect(region, origin) else {
            continue;
        };
        let name = indexed.get(region.path.as_str()).map(|path| path.as_str());
        if root.is_named("strategicairview") {
            if name == Some("air_combat_wnd") {
                drawn.air_combat_rect = Some(rect);
            }
            if name == Some("details_wnd") {
                drawn.air_details_rect = Some(rect);
            }
            if let Some(action) = name.and_then(super::air_menu::Action::from_path) {
                drawn.air_controls.push((region.path.clone(), action));
            }
        }
        if !popup {
            match name {
                Some("dismiss_button") => drawn.dismiss = Some(rect),
                Some("open_details") => drawn.result_details_button = Some(rect),
                Some("attacker_details" | "defender_details") => {
                    drawn.result_details_rects.push(rect)
                }
                _ => {}
            }
        }
        if name.is_some_and(|path| {
            path.rsplit('/').next().is_some_and(|name| {
                name.eq_ignore_ascii_case("close") || name.eq_ignore_ascii_case("close_button")
            })
        }) {
            if popup {
                drawn.tactics_close = Some(rect);
            } else {
                drawn.close = Some(rect);
            }
        }
        if name.is_some_and(|name| name.eq_ignore_ascii_case("tactics_list_button")) {
            drawn.tactics_button = Some(rect);
        }
        if let Some(path) = name
            && let Some(container) = root.path_named(path).and_then(Element::container)
        {
            for (vertical, bar) in [
                (false, &container.horizontal_scrollbar),
                (true, &container.vertical_scrollbar),
            ] {
                if !bar.is_empty() {
                    drawn.lists.push((
                        rect,
                        ScrollTarget {
                            popup,
                            path: path.to_owned(),
                            vertical,
                        },
                    ));
                }
            }
        }
        if let Some(path) = name
            && root
                .path_named(path)
                .and_then(Element::text_box)
                .is_some_and(|text| !text.scrollbar_type.is_empty())
        {
            drawn.lists.push((
                rect,
                ScrollTarget {
                    popup,
                    path: path.to_owned(),
                    vertical: true,
                },
            ));
        }
        if let Some((vertical, direction)) = region.scroll
            && let Some(path) = region.owner.as_deref().and_then(|owner| indexed.get(owner))
        {
            drawn.scroll_buttons.push((
                rect,
                ScrollTarget {
                    popup,
                    path: path.to_owned(),
                    vertical,
                },
                direction,
            ));
        }
    }
}

#[cfg(test)]
mod tests;
