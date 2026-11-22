
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Listed {
    Icon,
    Browser,
    CurveGraph,
    DropDown,
    Scrollbar,
    ListBox,
    SmoothListBox,
    Overlapping,
    InstantText,
    GuiButton,
    MultiSprite,
    TextBox,
    EditBox,
    Checkbox,
    Window,
}

impl Listed {
    const BUILT: [Listed; 15] = [
        Listed::Icon,
        Listed::Browser,
        Listed::CurveGraph,
        Listed::DropDown,
        Listed::Scrollbar,
        Listed::ListBox,
        Listed::SmoothListBox,
        Listed::Overlapping,
        Listed::InstantText,
        Listed::GuiButton,
        Listed::MultiSprite,
        Listed::TextBox,
        Listed::EditBox,
        Listed::Checkbox,
        Listed::Window,
    ];

    fn of(kind: &Kind) -> Option<Listed> {
        Some(match kind {
            Kind::Icon => Listed::Icon,
            Kind::Browser => Listed::Browser,
            Kind::CurveGraph => Listed::CurveGraph,
            Kind::DropDown => Listed::DropDown,
            Kind::Scrollbar => Listed::Scrollbar,
            Kind::ListBox => Listed::ListBox,
            Kind::SmoothListBox => Listed::SmoothListBox,
            Kind::List => Listed::Overlapping,
            Kind::TextBox => Listed::InstantText,
            Kind::GuiButton => Listed::GuiButton,
            Kind::MultiSpriteButton => Listed::MultiSprite,
            Kind::TextSprite => Listed::TextBox,
            Kind::EditBox => Listed::EditBox,
            Kind::Checkbox => Listed::Checkbox,
            Kind::Window => Listed::Window,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Button,
    Icon,
    CheckBox,
    MultiSprite,
    InstantText,
    TextBox,
    Scrollbar,
    EditBox,
    ListBox {
        smooth: bool,
    },
    Overlapping,
    Browser,
    CurveGraph,
    DropDown,
    Window,
}

fn button_type(kind: &Kind) -> bool {
    matches!(
        kind,
        Kind::GuiButton | Kind::MultiSpriteButton | Kind::Checkbox | Kind::Button
    )
}

impl Class {
    fn of(list: Listed, found: &Element) -> Result<Class, Refusal> {
        let kind = &found.kind;
        let confused = |what: &str| {
            Err(Refusal::new(
                &found.path,
                RefusalKind::TypeConfusion,
                format!(
                    "is the type a windowType's {what} finds under its name: a {} (the name \
                     tree keeps the last child of a name, whatever its kind)",
                    kind.keyword()
                ),
            ))
        };
        Ok(match list {
            Listed::Icon | Listed::GuiButton if button_type(kind) => Class::Button,
            Listed::Icon | Listed::GuiButton if *kind == Kind::Icon => Class::Icon,
            Listed::Icon | Listed::GuiButton => {
                return confused(
                    "button or icon factory, which calls beyond its class's virtual methods",
                );
            }
            Listed::Checkbox if button_type(kind) => Class::CheckBox,
            Listed::Checkbox => {
                return confused(
                    "check box factory, which calls beyond its class's virtual methods",
                );
            }
            Listed::MultiSprite if button_type(kind) => Class::MultiSprite,
            Listed::MultiSprite => {
                return confused("multi-sprite factory, which reads a button template");
            }
            Listed::InstantText if *kind == Kind::TextBox => Class::InstantText,
            Listed::InstantText => {
                return confused("instant text factory, which reads an instant text template");
            }
            Listed::TextBox if *kind == Kind::TextSprite => Class::TextBox,
            Listed::TextBox => {
                return confused("text box factory, which reads a text box template");
            }
            Listed::Scrollbar if *kind == Kind::Scrollbar => Class::Scrollbar,
            Listed::Scrollbar => {
                return confused("scrollbar factory, which reads a scrollbar template");
            }
            Listed::EditBox if *kind == Kind::EditBox => Class::EditBox,
            Listed::EditBox => {
                return confused("edit box factory, which reads an edit box template");
            }
            Listed::ListBox | Listed::SmoothListBox
                if matches!(kind, Kind::ListBox | Kind::SmoothListBox) =>
            {
                Class::ListBox {
                    smooth: list == Listed::SmoothListBox,
                }
            }
            Listed::ListBox | Listed::SmoothListBox => {
                return confused("list box factory, which reads a list box template");
            }
            Listed::Overlapping if *kind == Kind::List => Class::Overlapping,
            Listed::Overlapping => {
                return confused(
                    "overlapping list factory, which reads an overlapping list template",
                );
            }
            Listed::Browser if *kind == Kind::Browser => Class::Browser,
            Listed::Browser => {
                return confused("browser factory, which reads a browser template");
            }
            Listed::CurveGraph if *kind == Kind::CurveGraph => Class::CurveGraph,
            Listed::CurveGraph => {
                return confused("curve graph factory, which reads a curve graph template");
            }
            Listed::DropDown if *kind == Kind::DropDown => Class::DropDown,
            Listed::DropDown => {
                return confused("drop-down factory, which reads a drop-down template");
            }
            Listed::Window if *kind == Kind::Window => Class::Window,
            Listed::Window => {
                return confused("window factory, which reads a window template");
            }
        })
    }
}

struct Built<'a> {
    list: Listed,
    name: &'a [u8],
    data: &'a Element,
    class: Class,
}

fn fields(window: &Element) -> Result<&native::Window, Refusal> {
    window.window().ok_or_else(|| {
        Refusal::new(
            &window.path,
            RefusalKind::Reader,
            format!("is a {}, not a windowType", window.kind.keyword()),
        )
    })
}

fn named<'a>(window: &'a Element, name: &[u8]) -> Option<&'a Element> {
    window
        .children
        .iter()
        .rev()
        .find(|child| child.is_named(name))
}

fn size(window: &Element, viewport: Option<[i32; 2]>) -> Result<[i32; 2], Refusal> {
    let own = fields(window)?;
    let mut out = [0; 2];
    for (axis, extent) in own.size.iter().enumerate() {
        out[axis] = match *extent {
            native::WindowExtent::Pixels(value) => value,
            native::WindowExtent::MaxResolution { less } => {
                let viewport = viewport.ok_or_else(|| {
                    Refusal::new(
                        &window.path,
                        RefusalKind::NoParent,
                        "has a max_resolution size, the GUI root's extent less its position, which the caller did not bind (Bindings::viewport)",
                    )
                })?;
                viewport[axis].wrapping_sub(less)
            }
        };
    }
    Ok(out)
}

pub fn fixed_window_origin(window: &Element, extent: [i32; 2]) -> Result<[i32; 2], Refusal> {
    let own = fields(window)?;
    let anchor = control_anchor(own.orientation, extent);
    Ok([
        own.position[0].wrapping_add(anchor[0]),
        own.position[1].wrapping_add(anchor[1]),
    ])
}

pub(super) fn background<'a>(
    window: &'a Element,
    gui_type: &dyn Fn(&[u8]) -> Option<&'a Element>,
) -> Result<Option<&'a Element>, Refusal> {
    let own = fields(window)?;
    if own.background.is_empty() {
        return Ok(None);
    }
    let name = &own.background;
    let found = match named(window, name) {
        Some(found) => found,
        None => {
            let Some(parent) = gui_type(&own.parent) else {
                return Err(Refusal::new(
                    &window.path,
                    RefusalKind::NullRead,
                    format!(
                        "names the background {:?}, which it does not hold, and its parent {:?} names no template: background lookup reads the null parent",
                        lossy(name),
                        lossy(&own.parent)
                    ),
                ));
            };
            if parent.kind != Kind::Window {
                return Err(Refusal::new(
                    &window.path,
                    RefusalKind::TypeConfusion,
                    format!(
                        "looks its background {:?} up in its parent {}, a {}, whose fields are read as a window template's name tree",
                        lossy(name),
                        parent.path,
                        parent.kind.keyword()
                    ),
                ));
            }
            named(parent, name).ok_or_else(|| {
                Refusal::new(
                    &window.path,
                    RefusalKind::NullRead,
                    format!(
                        "names the background {:?}, which neither it nor its parent {} holds: background construction reads the null type",
                        lossy(name),
                        parent.path
                    ),
                )
            })?
        }
    };
    if !button_type(&found.kind) {
        return Err(Refusal::new(
            &found.path,
            RefusalKind::TypeConfusion,
            format!(
                "is the background of {}, a {}, which background construction reads as a button template",
                window.path,
                found.kind.keyword()
            ),
        ));
    }
    Ok(Some(found))
}

fn build<'a>(window: &'a Element, own: &native::Window) -> (Vec<Built<'a>>, Refusals) {
    let mut built: Vec<Built> = Vec::new();
    let mut refused = Refusals::new();
    for list in Listed::BUILT {
        for (index, child) in window.children.iter().enumerate() {
            if Listed::of(&child.kind) != Some(list) || own.unlisted.contains(&index) {
                continue;
            }
            let name = child.name_bytes();
            let data = named(window, name).expect("the name tree holds every child's name");
            match Class::of(list, data) {
                Ok(class) => built.push(Built {
                    list,
                    name,
                    data,
                    class,
                }),
                Err(refusal) => refused.push(refusal),
            }
        }
    }
    (built, refused)
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FixedFind {
    Button,
    InstantTextBox,
    Icon,
    TextBox,
    CheckBox,
    EditBox,
    Browser,
    CurveGraph,
    DropDownBox,
    Scrollbar,
    OverlappingElementsBox,
    StandardListBox,
    SmoothListBox,
    Subwindow,
}

#[cfg(any(test, feature = "test-support"))]
pub fn fixed_window_find(
    window: &Element,
    find: FixedFind,
    name: impl AsRef<[u8]>,
) -> Option<&Element> {
    let name = name.as_ref();
    let own = window.window()?;
    let (built, _) = build(window, own);
    let same = |object: &&Built| object.name.eq_ignore_ascii_case(name);
    let first = |list: Listed| {
        built
            .iter()
            .filter(|object| object.list == list)
            .find(same)
            .map(|object| object.data)
    };
    let last = |list: Listed| {
        built
            .iter()
            .filter(|object| object.list == list)
            .rev()
            .find(same)
            .map(|object| object.data)
    };
    match find {
        FixedFind::Button => first(Listed::GuiButton).or_else(|| first(Listed::MultiSprite)),
        FixedFind::InstantTextBox => first(Listed::InstantText),
        FixedFind::Icon => first(Listed::Icon),
        FixedFind::TextBox => last(Listed::TextBox),
        FixedFind::CheckBox => last(Listed::Checkbox),
        FixedFind::EditBox => last(Listed::EditBox),
        FixedFind::Browser => first(Listed::Browser),
        FixedFind::CurveGraph => first(Listed::CurveGraph),
        FixedFind::DropDownBox => first(Listed::DropDown),
        FixedFind::Scrollbar => last(Listed::Scrollbar),
        FixedFind::OverlappingElementsBox => first(Listed::Overlapping),
        FixedFind::StandardListBox => last(Listed::ListBox),
        FixedFind::SmoothListBox => last(Listed::SmoothListBox),
        FixedFind::Subwindow => built
            .iter()
            .find(|object| object.list == Listed::Window && object.name == name)
            .map(|object| object.data),
    }
}

fn construction_failure(refusal: &Refusal) -> bool {
    matches!(
        refusal.kind,
        RefusalKind::UndefinedRead
            | RefusalKind::NullRead
            | RefusalKind::TypeConfusion
            | RefusalKind::DivideByZero
            | RefusalKind::Hang
    )
}

pub fn fixed_window_layout(
    window: &Element,
    resources: &dyn Resources,
    bindings: &Bindings,
) -> Result<Option<Layout>, Refusals> {
    fields(window).map_err(|refusal| vec![refusal])?;
    let gui_type = |name: &[u8]| resources.gui_type(name);
    let mut reader = reader_refusals(window);
    if let Ok(Some(found)) = background(window, &gui_type)
        && !std::ptr::eq(found, window)
        && !window
            .children
            .iter()
            .any(|child| std::ptr::eq(child, found))
    {
        reader.extend(reader_refusals(found));
    }
    if !reader.is_empty() {
        return Err(reader);
    }
    let extent = size(window, bindings.viewport).map_err(|refusal| vec![refusal])?;
    let walk = |hovered: Option<String>, boxes: Vec<String>| -> Result<Option<Walk>, Refusals> {
        let mut walk = Walk {
            hovered,
            boxes,
            ..Walk::new(resources, bindings)
        };
        let drawn = match walk.fixed(window, [0, 0], None, true) {
            Ok(drawn) => drawn,
            Err(refusal) => {
                walk.refusals.push(refusal);
                false
            }
        };
        let refused = walk.refused.take();
        walk.refusals.extend(refused);
        if !walk.refusals.is_empty() {
            return Err(walk.refusals);
        }
        Ok(drawn.then_some(walk))
    };
    let first = bindings.pointer.as_ref().map_or((None, Vec::new()), |pointer| {
        (pointer.drawn.clone(), pointer.drawn_boxes.clone())
    });
    let Some(mut drawn) = walk(first.0.clone(), first.1.clone())? else {
        return Ok(None);
    };
    if let Some(pointer) = &bindings.pointer {
        let (hovered, boxes, target) = match &pointer.over {
            Some(over) => (
                Some(over.clone()),
                Vec::new(),
                Some(super::Target {
                    path: over.clone(),
                    check_box: false,
                    checked: false,
                }),
            ),
            None => pointer_targets(&drawn.parts, &drawn.hits, &drawn.regions, pointer)?,
        };
        let hovered = match &pointer.pressed {
            Some(pressed) => hovered.filter(|hovered| hovered == pressed),
            None => hovered,
        };
        if (&hovered, &boxes) != (&first.0, &first.1) || pointer.pressed.is_some() {
            drawn = walk(hovered, boxes)?.expect("drawn in the first pass");
        }
        drawn.target = target;
    }
    let content = match background(window, &gui_type).map_err(|refusal| vec![refusal])? {
        None => extent,
        Some(found) => {
            let button = found
                .button()
                .expect("button fields, checked by background");
            drawn
                .sized_extent(&found.path, &button.sprite, extent)
                .ok_or_else(|| {
                    vec![Refusal::new(
                        &window.path,
                        RefusalKind::NullRead,
                        format!(
                            "has the background {}, whose sprite and the fallback are both undefined, so its drag button has no drawable, which the list box row extent reads through",
                            found.path
                        ),
                    )]
                })?
        }
    };
    Ok(Some(drawn.finish(content)))
}

impl Walk<'_> {
    fn fixed(
        &mut self,
        window: &Element,
        origin: [i32; 2],
        clip: Option<Clip>,
        draw: bool,
    ) -> Result<bool, Refusal> {
        if draw && !self.shown(window) {
            return Ok(false);
        }
        let own = fields(window)?;
        let extent = size(window, self.bindings.viewport)?;
        if draw && self.bindings.regions {
            self.regions.push(Region {
                path: window.path.clone(),
                owner: self.region_owner.clone(),
                background_index: None,
                scroll: None,
                slide: None,
                rect: [
                    origin[0] as f32,
                    origin[1] as f32,
                    extent[0] as f32,
                    extent[1] as f32,
                ],
                clip,
                hit: self
                    .bindings
                    .viewport
                    .map(|viewport| HitGeometry::rectangle(origin, extent, viewport, clip)),
                disabled: false,
            });
        }
        let (first_region, first_holder) = (self.regions.len(), self.holders.len());
        let resources = self.resources;
        if let Some(found) = background(window, &|name| resources.gui_type(name))?
            && draw
        {
            let button = found
                .button()
                .expect("button fields, checked by background");
            let at = [
                origin[0].wrapping_add(button.position[0]),
                origin[1].wrapping_add(button.position[1]),
            ];
            let drawn = self.built(window, Role::Background, |walk| {
                walk.drag_part(found, button, button.frame, at, Some(extent), clip)
            });
            if let Err(refusal) = drawn {
                self.refusals.push(Refusal {
                    detail: format!("background of {}: {}", window.path, refusal.detail),
                    ..refusal
                });
            }
        }
        let (built, refused) = build(window, own);
        self.refusals.extend(refused);
        let mut attached: Vec<usize> = Vec::new();
        for (index, child) in window.children.iter().enumerate() {
            if child.kind == Kind::MultiSpriteButton || own.unlisted.contains(&index) {
                continue;
            }
            if let Some(object) = attach_lookup(&built, child.name_bytes()) {
                attached.retain(|&other| other != object);
                attached.push(object);
            }
        }
        for (index, object) in built.iter().enumerate() {
            if draw && attached.contains(&index) && self.shown(object.data) {
                continue;
            }
            let mut scratch = Walk::new(self.resources, self.bindings);
            let drawn = scratch.fixed_object(object, origin, extent, None, true);
            self.refusals.extend(
                drawn
                    .err()
                    .into_iter()
                    .chain(scratch.refusals)
                    .filter(construction_failure),
            );
        }
        if !draw {
            return Ok(false);
        }
        let root = self.top.is_none();
        for object in attached {
            let object = &built[object];
            if !self.shown(object.data) {
                continue;
            }
            if root {
                self.top = window
                    .children
                    .iter()
                    .position(|child| std::ptr::eq(child, object.data));
            }
            if let Err(refusal) = self.fixed_object(object, origin, extent, clip, false) {
                self.refusals.push(refusal);
            }
        }
        if root {
            self.top = None;
        }
        if self.bindings.regions {
            for region in &mut self.regions[first_region..] {
                if let Some(hit) = &mut region.hit {
                    hit.inherit(1.0, origin);
                }
            }
            for holder in &mut self.holders[first_holder..] {
                if let Some(parents) = &mut holder.hit_scales {
                    parents.insert(0, (1.0, origin));
                }
            }
        }
        Ok(true)
    }

    fn fixed_object(
        &mut self,
        object: &Built,
        origin: [i32; 2],
        extent: [i32; 2],
        clip: Option<Clip>,
        construct: bool,
    ) -> Result<(), Refusal> {
        let data = object.data;
        match (object.class, &data.fields) {
            (Class::Button, Fields::Button(button)) => {
                self.button(data, button, origin, extent, clip)
            }
            (Class::Icon, Fields::Icon(icon)) => self.icon(data, icon, origin, extent, clip),
            (Class::CheckBox, Fields::Button(button)) => {
                self.check_box(data, button, origin, extent, clip)
            }
            (Class::MultiSprite, Fields::Button(button)) => {
                let multi = native::Button {
                    orientation: Some(Orientation::UpperLeft),
                    scale: 1.0,
                    rotation: 0.0,
                    frame: 1,
                    size: [0, 0],
                    border_size: [0, 0],
                    ..button.clone()
                };
                self.button(data, &multi, origin, [0, 0], clip)
            }
            (Class::InstantText, Fields::TextBox(text)) => {
                self.text_box(data, text, origin, extent, clip)
            }
            (Class::TextBox, Fields::TextSprite(text)) => {
                self.fixed_text_box(data, text, origin, extent, clip)
            }
            (Class::Scrollbar, Fields::Scrollbar(bar)) if construct => {
                self.plain(data, bar).map(|_| ())
            }
            (Class::Scrollbar, Fields::Scrollbar(bar)) => {
                self.scrollbar_child(data, bar, origin, clip)
            }
            (Class::EditBox, Fields::EditBox(edit)) => {
                self.edit_box(data, edit, origin, extent, clip)
            }
            (Class::ListBox { smooth }, Fields::ListBox(list)) => {
                self.list_box_as(data, list, smooth, origin, extent, clip)
            }
            (Class::Overlapping, Fields::List(_)) => {
                self.hold(data, origin, extent, clip, 1.0, None)
            }
            (Class::Browser, Fields::Browser(_)) if construct => Ok(()),
            (Class::Browser, Fields::Browser(_)) => self.browser(data),
            (Class::CurveGraph, Fields::Browser(_)) => self.curve_graph(data),
            (Class::DropDown, Fields::DropDown(drop)) => self.drop_down(data, drop, origin, clip),
            (Class::Window, Fields::Window(_)) => {
                let place = self
                    .bound(data)
                    .and_then(|bound| bound.position)
                    .map_or_else(|| fixed_window_origin(data, extent), Ok)?;
                let at = [
                    origin[0].wrapping_add(place[0]),
                    origin[1].wrapping_add(place[1]),
                ];
                self.fixed(data, at, clip, !construct).map(|_| ())
            }
            _ => unreachable!("Class::of matched the kind to the class"),
        }
    }

    fn fixed_text_box(
        &mut self,
        element: &Element,
        text: &native::TextSprite,
        origin: [i32; 2],
        extent: [i32; 2],
        clip: Option<Clip>,
    ) -> Result<(), Refusal> {
        let [w, h] = extent;
        let offset = match text.orientation.gui_value() {
            1 => [w, 0],
            2 => [0, h],
            3 => [w, h],
            4 if text.format == 2 => [
                (w / 2).wrapping_sub(((text.max_width as u32) >> 1) as i32),
                h / 2,
            ],
            4 => [w / 2, h / 2],
            5 => [w / 2, 0],
            6 => [w / 2, h],
            _ => [0, 0],
        };
        let at = [
            origin[0]
                .wrapping_add(text.position[0])
                .wrapping_add(offset[0]),
            origin[1]
                .wrapping_add(text.position[1])
                .wrapping_add(offset[1]),
        ];
        let Some(shown) = self.authored_text(element, &text.text) else {
            return Ok(());
        };
        self.text_box_sprite(
            element,
            text,
            &shown,
            [text.max_width, text.max_height],
            at,
            clip,
        )?;
        Ok(())
    }
}

fn attach_lookup(built: &[Built], name: &[u8]) -> Option<usize> {
    let same = |object: &Built| object.name.eq_ignore_ascii_case(name);
    let first = |list: Listed| {
        built
            .iter()
            .position(|object| object.list == list && same(object))
    };
    let last = |list: Listed| {
        built
            .iter()
            .rposition(|object| object.list == list && same(object))
    };
    last(Listed::Scrollbar)
        .or_else(|| first(Listed::GuiButton))
        .or_else(|| first(Listed::MultiSprite))
        .or_else(|| last(Listed::TextBox))
        .or_else(|| first(Listed::InstantText))
        .or_else(|| last(Listed::EditBox))
        .or_else(|| first(Listed::Icon))
        .or_else(|| first(Listed::Browser))
        .or_else(|| first(Listed::CurveGraph))
        .or_else(|| last(Listed::Checkbox))
        .or_else(|| last(Listed::ListBox))
        .or_else(|| last(Listed::SmoothListBox))
        .or_else(|| {
            built
                .iter()
                .position(|object| object.list == Listed::Window && object.name == name)
        })
        .or_else(|| first(Listed::Overlapping))
}
