//! Retained combat windows and code-added entries through the native GUI walk.
use super::{Drawn, ScrollTarget, Selection, TooltipHit, Values, binding::Bindings};
use crate::map::{
    counters::gui_windows::{self, NativeWindows},
    native_gui::{
        self, Layout, Part, Region, Resources, SpriteKind, SpriteSetter,
        lifecycle::{Instance, Tick},
    },
};
use installed_content::{
    crt::truncate_f32,
    gui::native::{Element, Kind},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

/// Recorded flag keys resolved for this presentation, over the native draw resources.
pub(super) trait CombatResources: Resources + native_gui::Sheets {
    fn flag_index(&self, key: &str) -> Result<i32, String>;
    fn flag_texture(&self, key: &str, small: bool) -> Option<native_gui::Texture>;
}

#[derive(Default)]
pub(super) struct Windows {
    built: Option<(NativeWindows, [i32; 2])>,
    root: Option<Retained>,
    popup: Option<Retained>,
    pending: Vec<ScrollAction>,
}

struct Retained {
    name: String,
    instance: Instance,
    entries: BTreeMap<String, Retained>,
    pending: Vec<ScrollAction>,
    prepared_generation: Option<u64>,
    places: BTreeMap<String, Vec<(usize, [i32; 2])>>,
    bind_refused: BTreeMap<String, usize>,
    bind_failed: bool,
}

struct ScrollAction {
    target: ScrollTarget,
    direction: i32,
    wheel: bool,
}

struct Row<'a> {
    key: String,
    template: &'a str,
    values: &'a Values,
    /// Runtime size and optional explicit pixels; None pixels use authored grid slots.
    explicit: Option<(Option<[f32; 2]>, [f32; 2])>,
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

struct Group<'a> {
    path: String,
    rows: Vec<Row<'a>>,
}

struct Prepared {
    layout: Layout,
    items: Vec<Vec<Part>>,
    hovers: Vec<Hover>,
}

struct Hover {
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

fn template<'a>(windows: &'a NativeWindows, name: &str) -> Result<&'a Element, String> {
    windows
        .gui_type(name.as_bytes())
        .ok_or_else(|| format!("the content stack declares no {name} template (142247660)"))
}

fn ensure<'a>(
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
    fn active(&self) -> bool {
        self.instance.active() || self.entries.values().any(Self::active)
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
        })
    }

    fn bind(
        &mut self,
        root: &Element,
        values: &Values,
        size: Option<[f32; 2]>,
        resources: &impl CombatResources,
        refused: &mut BTreeMap<String, usize>,
    ) -> Option<[i32; 2]> {
        apply(&mut self.instance, root, values, resources, refused);
        let mut size = size?.map(truncate_f32);
        // Escaping entries grow by the bound escape bar's queried height (1417DBBE0).
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
                // The native updater explicitly centres these controls after its flag-0 resize (1417DBBE0).
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
    fn prepare<'v>(
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
                self.bind(root, values, None, resources, refused);
            }
            let mut places = BTreeMap::new();
            let mut used = BTreeSet::new();
            let first = laid(&self.instance, root, resources, refused, false);
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
                                    Some((
                                        offset.map(truncate_f32),
                                        size.unwrap_or_else(|| raw_size.map(truncate_f32)),
                                    ))
                                })
                                .collect();
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
                                    // 1417D05D0 uses automatic slots for sunk entries but
                                    // 1417DC8D0 resizes each to its sprite before finalize.
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
                                                "entry exceeds the authored grid slots (1402DE850)",
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
                // Land refill finalizes, then synchronizes including hidden children (1415CA230/1415CA460).
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
        for action in self.pending.drain(..) {
            let Some(container) = root.path_named(&action.target.path) else {
                count(
                    refused,
                    &action.target.path,
                    "scroll target is absent from the native window",
                );
                continue;
            };
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
        // Input precedes one smoothing tick in ProcessInputEvents@142247750.
        if advance && let Err(why) = self.instance.tick(root, resources, tick) {
            count(refused, &self.name, why);
            return None;
        }
        let layout = laid(&self.instance, root, resources, refused, true)?;
        let mut hovered = if hovers {
            hover_regions(root, &layout.regions, values)
        } else {
            Vec::new()
        };
        let mut items = vec![Vec::new(); layout.holders.len()];
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
                    refused,
                ) else {
                    continue;
                };
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
        // before later siblings (142226BD0, 142226CC0, 142225EC0).
        hovered.sort_by_key(|hit| hit.order);
        Some(Prepared {
            layout,
            items,
            hovers: hovered,
        })
    }
}

fn laid(
    instance: &Instance,
    root: &Element,
    resources: &impl CombatResources,
    refused: &mut BTreeMap<String, usize>,
    noted: bool,
) -> Option<Layout> {
    match instance.layout_with_regions(root, resources, None, true) {
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

/// 141C5EF70 / 141C60E40 select each assigned-trait entry's template.
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

// Native name lookup folds ASCII case (tolower@14254F1D0); Values keep their producer keys.
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
        }
        let authored = element
            .icon()
            .map(|icon| icon.sprite.as_slice())
            .or_else(|| element.button().map(|button| button.sprite.as_slice()));
        let sprite = widget
            .and_then(|widget| widget.sprite.as_deref())
            .map(str::as_bytes)
            .or(authored);
        if let Some(sprite) = sprite {
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
        let progress = match (widget.fill, info.as_ref().map(|info| &info.kind)) {
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
                bound.color = widget.opacity.map(|opacity| [1.0, 1.0, 1.0, opacity]);
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

fn region_rect(region: &Region, origin: [i32; 2]) -> Option<[f32; 4]> {
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

// ReloadGui binds these native templates (1415C5900, 1417D7450, 1417C71F0).
pub(super) fn window_name(selection: Selection) -> &'static str {
    match selection {
        Selection::Land(_) => "landcombatview",
        Selection::Naval(_) => "navalcombatview",
        Selection::NavalResult(_) => "navalcombatresultswindow",
    }
}

fn hover_regions(root: &Element, regions: &[Region], values: &Values) -> Vec<Hover> {
    let indexed = bound_values(root, values);
    // CStandardGridBoxItem forwards its provider through the entry container
    // (CTacticsListEntry141C5A870). Children with their own handler keep it.
    let provider = values.get("").filter(|widget| widget.hover_owned);
    // CInstantTextObject::HitTest@1422D8240 rejects the constructor's
    // always-transparent bit (1422B50A0 -> 1422D7A00), even with a handler.
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

    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw(
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
            // SetCombat and list refill preserve producer membership order (1415C7520/1415CA230).
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
            &mut drawn.refused,
        ) {
            drawn.rect = [
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
                controls(root, &prepared.layout.regions, origin, &mut drawn, false);
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
            // The possible-tactics popup sits ten pixels right of the combat window (1415C7D20).
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
                &mut drawn.refused,
            ) {
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
        key: index.to_string(),
        // CSunkShipIcon@1417CEF80 and CFexShipIcon@1417CE2A0 share this template.
        template: "navalcombat_ship_entry",
        values: &row.values,
        explicit: Some((row.offset, row.size)),
    }
}

fn draw_prepared(
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

fn append_hovers(target: &mut Vec<TooltipHit>, hovers: Vec<Hover>, origin: [i32; 2]) {
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
