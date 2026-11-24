
use super::*;
use content_reader_gui::gui::native::{Element, Kind};
use viewer_gpu::{camera::Camera, icon_tracks::{IconAnchor, IconFamily}};
use viewer_native_gui::{
    gui_resources::{count_noted, count_refused},
    icon_windows,
    native_gui::{self, Hit, HitGeometry, SpriteSetter, lifecycle::{Instance, Tick}},
    native_resources::Context,
    native_windows::NativeWindows,
    sprites::{self, Batch},
};

pub struct IntelInputs<'a> {
    pub anchor: &'a dyn Fn(u32) -> Option<[f32; 3]>,
    pub camera: Camera,
    pub viewport: [u32; 2],
    pub ui_scale: f32,
    pub operatives: bool,
}

#[derive(Clone, Debug)]
struct Placed {
    index: usize,
    screen: [f32; 2],
    anchor: IconAnchor,
}

#[derive(Clone, Debug, Default)]
pub struct PreparedIntel {
    icons: Arc<IntelIcons>,
    placed: Vec<Placed>,
    pub report: IntelReport,
}

pub fn prepare(
    assets: &IntelAssets,
    inputs: &IntelInputs<'_>,
    icons: &Arc<IntelIcons>,
) -> PreparedIntel {
    let mut out = PreparedIntel {
        icons: Arc::clone(icons),
        ..PreparedIntel::default()
    };
    let Ok(defines) = &assets.defines else {
        return out;
    };
    let free = inputs.camera.free().is_some();
    if !inputs.operatives || !(free || values::visible(inputs.camera.height, defines)) {
        return out;
    }
    let cutoff = defines.cutoff();
    out.report = icons.report.clone();
    let viewport =
        viewer_native_gui::gui_resources::viewport(inputs.viewport, inputs.ui_scale);
    let mut placed = Vec::with_capacity(icons.states.len());
    for (index, icon) in icons.states.iter().enumerate() {
        let world = match icon.anchor {
            Anchor::Province(province) => match (inputs.anchor)(province) {
                Some(world) => world,
                None => {
                    out.report.note(
                        "a state's anchor province has no label anchor, so its icon is not drawn",
                    );
                    continue;
                }
            },
            Anchor::Centre(world) => world,
        };
        let Some(projection) = inputs
            .camera
            .project_map_icon(world, inputs.viewport, viewport)
        else {
            continue;
        };
        placed.push((
            projection.depth,
            Placed {
                index,
                screen: projection.position,
                anchor: IconAnchor {
                    world,
                    cutoff,
                    family: IconFamily::Intel,
                    moves: None,
                },
            },
        ));
    }
    placed.sort_by(|a, b| b.0.total_cmp(&a.0));
    out.placed = placed.into_iter().map(|(_, placed)| placed).collect();
    out
}

#[derive(Debug)]
struct Retained {
    root: Instance,
    entries: Vec<(EntryKey, EntryKind, Instance, BTreeSet<String>)>,
}

#[derive(Debug, Default)]
pub struct IntelView {
    built: Option<(NativeWindows, [i32; 2])>,
    containers: HashMap<u32, Retained>,
    last: Option<f32>,
    active: bool,
}

impl IntelView {
    pub fn running(&self) -> bool {
        self.active
    }

    fn same<R>(&self, context: &Context<'_, R>) -> bool {
        self.built.as_ref().is_some_and(|(windows, viewport)| {
            windows.same(context.windows) && *viewport == context.viewport
        })
    }

    pub fn take_running(&mut self) -> bool {
        std::mem::take(&mut self.active)
    }
}

#[derive(Clone, Debug)]
pub struct IntelHit {
    pub shapes: Vec<HitGeometry>,
    pub layer: u64,
    pub tooltip: Arc<Tooltip>,
}

impl IntelHit {
    pub fn contains(&self, point: [f32; 2]) -> bool {
        self.shapes.iter().any(|shape| shape.contains(point))
    }

    pub fn bounds(&self) -> [f32; 4] {
        let mut out: Option<[f32; 4]> = None;
        for [x, y, w, h] in self.shapes.iter().filter_map(HitGeometry::bounds) {
            out = Some(match out {
                None => [x, y, w, h],
                Some([ox, oy, ow, oh]) => {
                    let (left, top) = (ox.min(x), oy.min(y));
                    let (right, bottom) = ((ox + ow).max(x + w), (oy + oh).max(y + h));
                    [left, top, right - left, bottom - top]
                }
            });
        }
        out.unwrap_or_default()
    }
}

pub fn hit_at(hits: &[IntelHit], point: [f32; 2]) -> Option<&IntelHit> {
    hit_seen(hits, |_| Some((point, 0)))
}

pub fn hit_seen(
    hits: &[IntelHit],
    seen: impl Fn(&IntelHit) -> Option<([f32; 2], u32)>,
) -> Option<&IntelHit> {
    let mut top: Option<((u32, u64), &IntelHit)> = None;
    for hit in hits {
        let Some((point, rank)) = seen(hit) else {
            continue;
        };
        if hit.contains(point) && top.is_none_or(|(key, _)| (rank, hit.layer) >= key) {
            top = Some(((rank, hit.layer), hit));
        }
    }
    top.map(|(_, hit)| hit)
}

struct Windows<'w> {
    root: &'w Element,
    list: &'w Element,
    entries: [Result<&'w Element, String>; 4],
}

fn kind_index(kind: EntryKind) -> usize {
    match kind {
        EntryKind::Shield => 0,
        EntryKind::Operative => 1,
        EntryKind::Operation => 2,
        EntryKind::More => 3,
    }
}

fn find_windows(native: &NativeWindows) -> Result<Windows<'_>, String> {
    let container = |name: &str| {
        native
            .gui_type(name.as_bytes())
            .filter(|window| window.kind == Kind::Container)
            .ok_or_else(|| format!("no .gui file declares the containerWindowType {name}"))
    };
    let root = container(CONTAINER_WINDOW)?;
    let list = root
        .find_list_recursive(ENTRIES_LIST)
        .ok_or_else(|| format!("{CONTAINER_WINDOW} declares no list {ENTRIES_LIST}"))?;
    Ok(Windows {
        root,
        list,
        entries: [
            EntryKind::Shield,
            EntryKind::Operative,
            EntryKind::Operation,
            EntryKind::More,
        ]
        .map(|kind| container(kind.window())),
    })
}

fn find<'e>(window: &'e Element, name: &str) -> Option<&'e Element> {
    let mut stack: Vec<&Element> = window.nested().into_iter().rev().collect();
    while let Some(element) = stack.pop() {
        if element.is_named(name) {
            return Some(element);
        }
        stack.extend(element.nested().into_iter().rev());
    }
    None
}

fn by_path<'e>(window: &'e Element, path: &str) -> Option<&'e Element> {
    if window.path == path {
        return Some(window);
    }
    window
        .nested()
        .into_iter()
        .find_map(|nested| by_path(nested, path))
}

fn transparent(element: &Element) -> bool {
    element
        .button()
        .is_some_and(|button| button.always_transparent)
        || element.icon().is_some_and(|icon| icon.always_transparent)
        || element
            .text_box()
            .is_some_and(|text| text.always_transparent)
}

impl Write {
    fn child(&self) -> &'static str {
        match self {
            Self::Shown(name, _)
            | Self::Sprite(name, _)
            | Self::Portrait(name, _)
            | Self::Frame(name, _)
            | Self::Text(name, _)
            | Self::Color(name, _)
            | Self::Progress(name, _)
            | Self::Enabled(name, _)
            | Self::Withheld(name, _) => name,
        }
    }
}

fn apply(
    instance: &mut Instance,
    withheld: &mut BTreeSet<String>,
    template: &Element,
    writes: &[Write],
    resources: &dyn native_gui::Resources,
    report: &mut IntelReport,
) -> Result<(), String> {
    for write in writes
        .iter()
        .filter(|write| !matches!(write, Write::Withheld(..)))
        .chain(
            writes
                .iter()
                .filter(|write| matches!(write, Write::Withheld(..))),
        )
    {
        let name = write.child();
        let Some(child) = find(template, name) else {
            report.note(format!(
                "{} declares no {name}, which the update writes",
                template.path
            ));
            continue;
        };
        let setter = if child.kind == Kind::Button {
            SpriteSetter::ButtonStandard
        } else {
            SpriteSetter::Icon
        };
        match write {
            Write::Withheld(..) => {}
            Write::Shown(..) => {
                withheld.remove(&child.path);
            }
            _ => {
                if withheld.remove(&child.path) {
                    instance.set_hide_flag(template, child, false)?;
                    instance.show(template, child)?;
                }
            }
        }
        match write {
            Write::Shown(_, true) => {
                instance.set_hide_flag(template, child, false)?;
                instance.show(template, child)?;
            }
            Write::Shown(_, false) => {
                instance.hide(template, child)?;
                instance.set_hide_flag(template, child, true)?;
            }
            Write::Withheld(_, why) => {
                instance.hide(template, child)?;
                instance.set_hide_flag(template, child, true)?;
                withheld.insert(child.path.clone());
                report.note(format!("{} is hidden: {why}", child.path));
            }
            Write::Sprite(_, sprite) => {
                if native_gui::sprite_resolves(sprite, resources) {
                    instance.set_sprite(template, child, sprite, setter)?;
                } else {
                    report.note(format!(
                        "{} keeps its sprite: no interface file loads {}",
                        child.path,
                        String::from_utf8_lossy(sprite)
                    ));
                }
            }
            Write::Portrait(_, sprite) => {
                instance.set_sprite(
                    template,
                    child,
                    OPERATIVE_UNKNOWN_PORTRAIT.as_bytes(),
                    setter,
                )?;
                instance.control(template, child)?.texture = resources.portrait_texture(sprite);
            }
            Write::Frame(_, frame) => instance.set_frame(template, child, *frame)?,
            Write::Text(_, text) => instance.set_text(template, child, text, resources)?,
            Write::Color(_, color) => instance.control(template, child)?.color = Some(*color),
            Write::Progress(_, value) => instance.control(template, child)?.progress = Some(*value),
            Write::Enabled(_, enabled) => {
                instance.control(template, child)?.enabled = Some(*enabled)
            }
        }
    }
    Ok(())
}

fn shapes(
    template: &Element,
    regions: &[native_gui::Region],
    origin: [i32; 2],
) -> Vec<HitGeometry> {
    regions
        .iter()
        .filter(|region| region.path != template.path && region.owner.is_none())
        .filter_map(|region| {
            let element = by_path(template, &region.path)?;
            if element.kind == Kind::Container || transparent(element) {
                return None;
            }
            let hit = region.hit.as_ref()?;
            (hit.form != Hit::Through).then(|| hit.shifted(origin))
        })
        .collect()
}

impl PreparedIntel {
    #[expect(clippy::too_many_arguments, reason = "the assets and context, the priority and view, and the batch, hits and report it fills")]
    pub fn draw(
        &self,
        assets: &IntelAssets,
        context: &Context<'_, impl native_gui::Resources + native_gui::Sheets>,
        priority: i8,
        view: &mut IntelView,
        batch: &mut Batch,
        hits: &mut Vec<IntelHit>,
        report: &mut IntelReport,
    ) -> usize {
        hits.clear();
        let Ok(defines) = &assets.defines else {
            return 0;
        };
        if self.placed.is_empty() {
            view.containers.clear();
            return 0;
        }
        let windows = match find_windows(context.windows) {
            Ok(windows) => windows,
            Err(why) => {
                *report
                    .reasons
                    .entry(format!("{CONTAINER_WINDOW} is not drawn: {why}"))
                    .or_default() += self.placed.len();
                return 0;
            }
        };
        if !view.same(context) {
            if context.time.is_none() {
                return 0;
            }
            *view = IntelView {
                built: Some((context.windows.clone(), context.viewport)),
                ..IntelView::default()
            };
        }
        let size = match native_gui::container_frame(windows.root, Some(context.viewport)) {
            Ok(frame) => frame.size,
            Err(why) => {
                report.note(format!("{CONTAINER_WINDOW} has no size: {why}"));
                return 0;
            }
        };
        let tick = context
            .time
            .map(|time| icon_windows::frame(&mut view.last, time));
        let mut drawn = 0;
        let mut shown = BTreeSet::new();
        for (ordinal, placed) in self.placed.iter().enumerate() {
            let icon = &self.icons.states[placed.index];
            shown.insert(icon.state);
            let point = values::container_point(placed.screen, size, defines.offset);
            let layer = sprites::layer(priority, ordinal as u32, 0);
            batch.at_anchor(Some(placed.anchor));
            match self.present(
                &windows, context, view, icon, point, tick, layer, batch, hits, report,
            ) {
                Ok(true) => drawn += 1,
                Ok(false) => {}
                Err(why) => {
                    view.containers.remove(&icon.state);
                    report.note(format!("{CONTAINER_WINDOW} is not drawn: {why}"));
                }
            }
        }
        batch.at_anchor(None);
        view.containers.retain(|state, _| shown.contains(state));
        drawn
    }

    #[expect(clippy::too_many_arguments, reason = "the windows and context, the view, the icon and its placement, the frame's tick and layer, and what it fills")]
    fn present(
        &self,
        windows: &Windows<'_>,
        context: &Context<'_, impl native_gui::Resources + native_gui::Sheets>,
        view: &mut IntelView,
        icon: &StateIcon,
        point: [i32; 2],
        tick: Option<Tick>,
        layer: u64,
        batch: &mut Batch,
        hits: &mut Vec<IntelHit>,
        report: &mut IntelReport,
    ) -> Result<bool, String> {
        let resources: &dyn native_gui::Resources = context.resources;
        let root = windows.root;
        if let std::collections::hash_map::Entry::Vacant(entry) = view.containers.entry(icon.state)
        {
            let mut instance = Instance::construct(root, context.viewport, resources)?;
            instance.hide(root, root)?;
            instance.take_held();
            entry.insert(Retained {
                root: instance,
                entries: Vec::new(),
            });
        }
        let retained = view
            .containers
            .get_mut(&icon.state)
            .expect("inserted above");
        retained.root.set_position(root, root, point)?;
        if !retained.root.drawn() {
            retained.root.gui_show(root, root)?;
        }
        retained.root.take_held();
        let mut kept = std::mem::take(&mut retained.entries);
        let mut entries = Vec::with_capacity(icon.entries.len());
        let mut templates = Vec::with_capacity(icon.entries.len());
        for entry in &icon.entries {
            let template = match &windows.entries[kind_index(entry.kind)] {
                Ok(template) => *template,
                Err(why) => {
                    report.note(format!("an entry is not drawn: {why}"));
                    continue;
                }
            };
            let instance = match kept
                .iter()
                .position(|(key, kind, _, _)| *key == entry.key && *kind == entry.kind)
            {
                Some(at) => {
                    let (_, _, instance, withheld) = kept.swap_remove(at);
                    (instance, withheld)
                }
                None => {
                    let mut instance = Instance::construct(template, context.viewport, resources)?;
                    instance.gui_show(template, template)?;
                    (instance, BTreeSet::new())
                }
            };
            let (instance, withheld) = instance;
            entries.push((entry.key.clone(), entry.kind, instance, withheld));
            templates.push((template, entry));
        }
        for ((_, _, instance, withheld), (template, entry)) in entries.iter_mut().zip(&templates) {
            apply(
                instance,
                withheld,
                template,
                &entry.writes,
                resources,
                report,
            )?;
        }
        let laid = retained
            .root
            .layout(root, resources, None)
            .map_err(|refusals| describe(&refusals))?;
        let holder = laid.as_ref().and_then(|layout| {
            layout
                .holders
                .iter()
                .find(|holder| holder.path == windows.list.path)
                .cloned()
        });
        let mut painted = Vec::new();
        if let (Some(holder), Some((first, _))) = (&holder, templates.first()) {
            let entry_size = native_gui::container_frame(first, Some(context.viewport))?.size;
            painted = native_gui::list_places(
                windows.list,
                holder.content,
                entry_size,
                templates.len(),
                holder.position,
            )?;
            for &(index, place) in &painted {
                let (template, _) = templates[index];
                entries[index].2.set_position(template, template, place)?;
            }
        }
        if let Some(tick) = tick {
            retained.root.tick(root, resources, tick)?;
            for ((_, _, instance, _), (template, _)) in entries.iter_mut().zip(&templates) {
                instance.tick(template, resources, tick)?;
            }
        }
        view.active |=
            retained.root.active() || entries.iter().any(|(_, _, instance, _)| instance.active());
        let layout = retained
            .root
            .layout(root, resources, None)
            .map_err(|refusals| describe(&refusals));
        retained.entries = entries;
        let Some(layout) = layout? else {
            return Ok(false);
        };
        count_noted(&mut report.reasons, CONTAINER_WINDOW, &layout.diagnostics);
        let mut items = vec![Vec::new(); layout.holders.len()];
        let mut entry_hits = Vec::new();
        let origin = retained.root.position();
        if let Some(at) = layout
            .holders
            .iter()
            .position(|holder| holder.path == windows.list.path)
        {
            let holder = &layout.holders[at];
            for &(index, _) in &painted {
                let (template, entry) = templates[index];
                let instance = &retained.entries[index].2;
                if !instance.drawn() {
                    continue;
                }
                match instance.layout_with_regions(template, resources, None, true) {
                    Ok(Some(laid)) => {
                        count_noted(&mut report.reasons, entry.kind.window(), &laid.diagnostics);
                        items[at].extend(holder.place(instance.position(), &laid));
                        let regions = holder.place_regions(instance.position(), &laid.regions);
                        entry_hits.push(IntelHit {
                            shapes: shapes(template, &regions, origin),
                            layer,
                            tooltip: Arc::clone(&entry.tooltip),
                        });
                    }
                    Ok(None) => {}
                    Err(refusals) => {
                        count_refused(&mut report.reasons, entry.kind.window(), &refusals)
                    }
                }
            }
        }
        batch.at_layer(layer);
        if let Err(refusal) =
            native_gui::draw_composed(&layout, &items, origin, context.resources, batch)
        {
            count_refused(
                &mut report.reasons,
                CONTAINER_WINDOW,
                std::slice::from_ref(&refusal),
            );
            return Ok(false);
        }
        hits.extend(entry_hits);
        Ok(true)
    }
}

fn describe(refusals: &[native_gui::Refusal]) -> String {
    refusals
        .iter()
        .map(|refusal| refusal.to_string())
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(any(test, feature = "test-support"))]
impl PreparedIntel {
    pub fn test_placed(icons: Arc<IntelIcons>, screens: &[[f32; 2]]) -> Self {
        Self {
            placed: screens
                .iter()
                .enumerate()
                .map(|(index, &screen)| Placed {
                    index,
                    screen,
                    anchor: IconAnchor {
                        world: [0.0; 3],
                        cutoff: f32::INFINITY,
                        family: IconFamily::Intel,
                        moves: None,
                    },
                })
                .collect(),
            report: icons.report.clone(),
            icons,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use viewer_native_gui::native_gui::Resources as _;
    use viewer_native_gui::test_support::gui;

    #[test]
    fn operative_portrait_setter_uses_its_own_fallback_and_recovers_after_withheld() {
        content_reader_script::test_game::enter();
        let windows = gui::windows(br#"guiTypes = {
            containerWindowType = { name = operative_map_icon_entry size = { width = 68 height = 48 }
                iconType = { name = portrait spriteType = GFX_even }
            }
        }"#);
        let template = windows.gui_type(OPERATIVE_WINDOW.as_bytes()).unwrap();
        let mut resources = gui::fixture();
        resources.add_sprite(OPERATIVE_UNKNOWN_PORTRAIT, [8, 8], 1);
        let mut instance = Instance::construct(template, [2048, 1152], &resources).unwrap();
        instance.gui_show(template, template).unwrap();
        let mut withheld = BTreeSet::new();
        for portrait in [Vec::new(), b"undeclared".to_vec(), b"GFX_even".to_vec()] {
            apply(
                &mut instance,
                &mut withheld,
                template,
                &[Write::Withheld("portrait", "missing input".into())],
                &resources,
                &mut IntelReport::default(),
            )
            .unwrap();
            apply(
                &mut instance,
                &mut withheld,
                template,
                &[Write::Portrait("portrait", portrait.clone())],
                &resources,
                &mut IntelReport::default(),
            )
            .unwrap();
            let layout = instance
                .layout(template, &resources, None)
                .unwrap()
                .unwrap();
            if portrait == b"GFX_even" {
                assert!(
                    layout.parts.iter().any(|part| matches!(
                        &part.kind, native_gui::PartKind::Cell { texture, .. }
                        if *texture == resources.portrait_texture(&portrait).unwrap().texture
                    )),
                    "{portrait:?}: {:?}",
                    layout.parts
                );
            } else {
                assert!(
                    layout.parts.iter().any(|part| matches!(
                        &part.kind, native_gui::PartKind::Sprite { sprite, .. }
                        if sprite == OPERATIVE_UNKNOWN_PORTRAIT.as_bytes()
                    )),
                    "{portrait:?}: {:?}",
                    layout.parts
                );
            }
        }
    }
}
