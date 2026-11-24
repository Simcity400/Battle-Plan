use super::tooltips::Details;
use content_reader_gui::gui::native;
use std::collections::BTreeMap;
use viewer_native_gui::{
    gui_resources::{hit_rectangle, hit_rectangles},
    native_gui::{self, SpriteSetter, lifecycle::Instance},
    native_resources::Context,
    native_windows::{RAID_DETAILS_ITEM_WINDOW, RAID_DETAILS_WINDOW, RAID_TYPE_ICON_WINDOW},
    sprites::Batch,
};

struct Controls<'a> {
    root: &'a native::Element,
    panel: &'a native::Element,
    header: &'a native::Element,
    list: &'a native::Element,
    item: &'a native::Element,
    name: &'a native::Element,
    grid: &'a native::Element,
    icon: &'a native::Element,
    button: &'a native::Element,
}

fn controls<'a>(context: &Context<'a, impl native_gui::Resources + native_gui::Sheets>) -> Result<Controls<'a>, String> {
    let template = |name: &str| {
        context
            .windows
            .gui_type(name.as_bytes())
            .ok_or_else(|| format!("{name} is absent from the recorded content stack"))
    };
    let find = |root: &'a native::Element, kind, name: &str| {
        root.find_recursive(kind, name.as_bytes())
            .ok_or_else(|| format!("{} has no {name}", root.path))
    };
    let root = template(RAID_DETAILS_WINDOW)?;
    let item = template(RAID_DETAILS_ITEM_WINDOW)?;
    let icon = template(RAID_TYPE_ICON_WINDOW)?;
    Ok(Controls {
        root,
        panel: find(root, native::Kind::Container, "raid_target_details")?,
        header: find(root, native::Kind::TextBox, "raid_targets_text")?,
        list: find(root, native::Kind::SmoothListBox, "raid_target_entries")?,
        item,
        name: find(item, native::Kind::TextBox, "raid_target_text")?,
        grid: find(item, native::Kind::Grid, "raid_type_icons_grid")?,
        icon,
        button: find(icon, native::Kind::Button, "raid_type_button")?,
    })
}

type Entry = (native_gui::Layout, Vec<(usize, Vec<native_gui::Part>)>);

pub(super) struct TypeHit {
    pub rect: [f32; 4],
    pub supported: bool,
    pub entry: usize,
    pub kind: usize,
}

impl TypeHit {
    pub(super) fn target(&self, details: &Details) -> Option<super::offers::TooltipTarget> {
        let row = details.entries.get(self.entry)?;
        Some(super::offers::TooltipTarget { scope: row.scope.clone(), types: vec![self.kind] })
    }
}

fn entry(
    controls: &Controls<'_>,
    entry: &super::tooltips::DetailEntry,
    viewport: [i32; 2],
    resources: &dyn native_gui::Resources,
    notes: &mut BTreeMap<String, usize>,
) -> Result<Option<Entry>, String> {
    let error = |refusals: native_gui::Refusals| format!("{refusals:?}");
    let mut item = Instance::construct(controls.item, viewport, resources)?;
    item.gui_show(controls.item, controls.item)?;
    match &entry.name {
        Ok(name) => item.set_text(controls.item, controls.name, name, resources)?,
        Err(why) => {
            item.hide(controls.item, controls.name)?;
            *notes.entry(why.clone()).or_default() += 1;
        }
    }
    let mut icons = Vec::new();
    match &entry.icons {
        Ok(buttons) => {
            for (kind, glyph, enabled) in buttons {
                let mut icon = Instance::construct(controls.icon, viewport, resources)?;
                icon.gui_show(controls.icon, controls.icon)?;
                if native_gui::sprite_resolves(glyph.as_bytes(), resources) {
                    icon.set_sprite(controls.icon, controls.button, glyph.as_bytes(), SpriteSetter::ButtonStandard)?;
                }
                icon.control(controls.icon, controls.button)?.enabled = Some(*enabled);
                icons.push((*kind, icon));
            }
        }
        Err(why) => {
            item.hide(controls.item, controls.grid)?;
            *notes.entry(why.clone()).or_default() += 1;
        }
    }
    let Some(layout) = item.layout(controls.item, resources, None).map_err(error)? else {
        return Ok(None);
    };
    let Some(at) = layout.holders.iter().position(|holder| holder.path == controls.grid.path) else {
        return Ok(Some((layout, Vec::new())));
    };
    let placed = item
        .finalize_grid(controls.item, controls.grid, &layout.holders[at], &vec![controls.icon; icons.len()])
        .map_err(error)?;
    let Some(mut layout) = item.layout(controls.item, resources, None).map_err(error)? else {
        return Ok(None);
    };
    let Some(at) = layout.holders.iter().position(|holder| holder.path == controls.grid.path) else {
        return Ok(Some((layout, Vec::new())));
    };
    let mut parts = vec![Vec::new(); layout.holders.len()];
    let mut buttons = Vec::with_capacity(icons.len());
    for ((kind, icon), place) in icons.iter().zip(placed.places()) {
        if let Some(laid) = icon.layout(controls.icon, resources, None).map_err(error)? {
            let placed = layout.holders[at].place(place, &laid);
            buttons.push((*kind, placed.iter().filter(|part| part.path == controls.button.path).cloned().collect()));
            parts[at].extend(placed);
        }
    }
    layout.parts = layout.composed_parts(&parts).cloned().collect();
    layout.holders.clear();
    Ok(Some((layout, buttons)))
}

pub(super) fn draw(
    context: &Context<'_, impl native_gui::Resources + native_gui::Sheets>,
    details: &Details,
    origin: [i32; 2],
    batch: &mut Batch,
    notes: &mut BTreeMap<String, usize>,
) -> Option<(Vec<[f32; 4]>, Vec<TypeHit>)> {
    match draw_inner(context, details, origin, batch, notes) {
        Ok(drawn) => drawn,
        Err(why) => {
            *notes.entry(format!("{RAID_DETAILS_WINDOW}: {why}")).or_default() += 1;
            None
        }
    }
}

fn draw_inner(
    context: &Context<'_, impl native_gui::Resources + native_gui::Sheets>,
    details: &Details,
    origin: [i32; 2],
    batch: &mut Batch,
    notes: &mut BTreeMap<String, usize>,
) -> Result<Option<(Vec<[f32; 4]>, Vec<TypeHit>)>, String> {
    let error = |refusals: native_gui::Refusals| format!("{refusals:?}");
    let controls = controls(context)?;
    let resources = context.resources;
    let viewport = context.viewport;
    let mut window = Instance::construct(controls.root, viewport, resources)?;
    window.gui_show(controls.root, controls.root)?;
    match &details.header {
        Ok(header) => window.set_text(controls.root, controls.header, header, resources)?,
        Err(why) => {
            window.hide(controls.root, controls.header)?;
            *notes.entry(why.clone()).or_default() += 1;
        }
    }
    let mut entries = Vec::with_capacity(details.entries.len());
    let mut buttons = Vec::with_capacity(details.entries.len());
    for (index, detail) in details.entries.iter().enumerate() {
        if let Some((laid, placed)) = entry(&controls, detail, viewport, resources, notes)? {
            entries.push(laid);
            buttons.push((index, placed));
        }
    }
    let native::Fields::ListBox(list) = &controls.list.fields else {
        return Err(format!("{} is not a list box", controls.list.path));
    };
    let list_at = list.position;
    let width = entries.first().map_or(0, |entry| entry.size[0]);
    let height = entries.iter().map(|entry| entry.size[1]).sum::<i32>();
    window
        .set_size(controls.root, controls.panel, [width + list_at[0], height + list_at[1]], resources)
        .map_err(error)?;
    let Some(layout) = window.layout(controls.root, resources, None).map_err(error)? else {
        return Err("the shown window lays out nothing".into());
    };
    let mut parts = vec![Vec::new(); layout.holders.len()];
    let mut hits = Vec::new();
    if let Some(at) = layout.holders.iter().position(|holder| holder.path == controls.list.path) {
        let sizes: Vec<[i32; 2]> = entries.iter().map(|entry| entry.size).collect();
        let mut bindings = native_gui::Bindings::default();
        bindings.viewport = Some(viewport);
        let rows = native_gui::widgets::list_box_rows(controls.list, &sizes, None, resources, &bindings).map_err(error)?;
        let holder = &layout.holders[at];
        for ((place, laid), (index, placed)) in rows.places.iter().zip(&entries).zip(&buttons) {
            let Some(place) = place else { continue };
            parts[at].extend(holder.place(*place, laid));
            for (kind, button) in placed {
                let alone = native_gui::Layout { parts: button.clone(), ..laid.clone() };
                let drawn = native_gui::Layout { parts: holder.place(*place, &alone), ..layout.clone() };
                if let Some(hit) = hit_rectangle(controls.button, &drawn, origin, resources, notes) {
                    hits.push(TypeHit { rect: hit.rect, supported: hit.supported, entry: *index, kind: *kind });
                }
            }
        }
        parts[at].extend(holder.place_scrollbar(&rows));
    }
    native_gui::draw_composed(&layout, &parts, origin, resources, batch).map_err(|why| format!("{why:?}"))?;
    Ok(Some((
        hit_rectangles(controls.panel, &layout, origin, resources, notes)
            .into_iter()
            .map(|hit| hit.rect)
            .collect(),
        hits,
    )))
}

