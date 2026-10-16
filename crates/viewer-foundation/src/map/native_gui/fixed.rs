//! Fixed windows used as list-box rows and nested `windowType` children.
//!
//! Size comes from the type; maximum-resolution axes subtract the stored value from
//! the GUI root extent. Position and orientation use the viewport for roots and the
//! outer window size for children, with a viewport-sized position object.
//!
//! Background lookup checks the type's name tree, then its parent template, and builds
//! the drag button first at the window size. Child kinds are built in fixed factory
//! order using the last type of each name. Declared order then attaches built objects
//! by fixed lookup priority; unmatched objects do not draw. The window draws and clips
//! nothing of its own and has no fade. A row's [`Layout::size`] uses its background
//! drawable extent when present, otherwise the window's own extent.

use super::*;

/// A `windowType`'s kind lists in construction order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Listed {
    /// `iconType`.
    Icon,
    /// `browserType`.
    Browser,
    /// `curveGraphType`.
    CurveGraph,
    /// `dropDownBoxType`.
    DropDown,
    /// `scrollbarType`.
    Scrollbar,
    /// `listBoxType`.
    ListBox,
    /// `smoothListboxType`.
    SmoothListBox,
    /// `OverlappingElementsBoxType`.
    Overlapping,
    /// `instantTextBoxType`.
    InstantText,
    /// `guiButtonType`.
    GuiButton,
    /// `multiSpriteButtonType`.
    MultiSprite,
    /// `textBoxType`.
    TextBox,
    /// `editBoxType`.
    EditBox,
    /// `checkboxType`.
    Checkbox,
    /// `windowType`, built after the others.
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

    /// The list a child of `kind` is read into.
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

/// The object a factory builds, by the class its constructor makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    /// Fixed-window button child.
    Button,
    /// An icon.
    Icon,
    /// A check box.
    CheckBox,
    MultiSprite,
    /// An instant text box.
    InstantText,
    /// A text box.
    TextBox,
    /// A scrollbar.
    Scrollbar,
    /// An edit box.
    EditBox,
    /// A standard or smooth list box.
    ListBox {
        smooth: bool,
    },
    /// An overlapping elements box.
    Overlapping,
    /// A browser.
    Browser,
    /// A curve graph.
    CurveGraph,
    /// A drop-down box.
    DropDown,
    /// A nested fixed window.
    Window,
}

/// Button-compatible types: `guiButtonType`, `multiSpriteButtonType`, `checkboxType`, `buttonType`.
fn button_type(kind: &Kind) -> bool {
    matches!(
        kind,
        Kind::GuiButton | Kind::MultiSpriteButton | Kind::Checkbox | Kind::Button
    )
}

impl Class {
    /// Build an object for `list` from `found`, the type held under the entry's name.
    /// Icon and button factories follow the found type's own class; every other
    /// factory interprets the type as its expected class.
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

/// One object a list entry builds.
struct Built<'a> {
    list: Listed,
    /// The entry's name as the list holds it.
    name: &'a [u8],
    /// The type its factory took.
    data: &'a Element,
    class: Class,
}

/// A `windowType`'s fields; `Err` for any other kind.
fn fields(window: &Element) -> Result<&native::Window, Refusal> {
    window.window().ok_or_else(|| {
        Refusal::new(
            &window.path,
            RefusalKind::Reader,
            format!("is a {}, not a windowType", window.kind.keyword()),
        )
    })
}

/// The last child held under the case-folded `name` in `window`'s name tree,
/// whatever its kind. Inserting a repeated name replaces the previous child.
fn named<'a>(window: &'a Element, name: &[u8]) -> Option<&'a Element> {
    window
        .children
        .iter()
        .rev()
        .find(|child| child.is_named(name))
}

/// The window's size: a `max_resolution` axis is the GUI root's extent less
/// the reader's stored value.
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

/// Position a fixed window against the GUI viewport for roots or the outer
/// window size for children. Add the authored position and orientation offset;
/// list rows then move it to their retained row position.
pub fn fixed_window_origin(window: &Element, extent: [i32; 2]) -> Result<[i32; 2], Refusal> {
    let own = fields(window)?;
    let anchor = control_anchor(own.orientation, extent);
    Ok([
        own.position[0].wrapping_add(anchor[0]),
        own.position[1].wrapping_add(anchor[1]),
    ])
}

/// Resolve a background from the window's own last child of that name, then
/// one level of its parent template. No background returns `None`; null or
/// incompatible reached templates refuse construction.
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

/// Every list's entries of `window`, in the constructor's order, each the object its factory
/// builds; the refusals of the factories given a type of another class.
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

/// Fixed-window typed name lookups search their constructed object lists
/// in kind-specific order. A list row's bound role selects the corresponding
/// object. The last matching declaration and each lookup's accepted kinds
/// remain distinct from container lookup.
#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FixedFind {
    /// the button arrays, else the multi-sprite arrays.
    Button,
    InstantTextBox,
    Icon,
    /// the `textBoxType` map.
    TextBox,
    CheckBox,
    EditBox,
    Browser,
    /// the curve graph arrays.
    CurveGraph,
    DropDownBox,
    /// the scrollbar map.
    Scrollbar,
    OverlappingElementsBox,
    StandardListBox,
    SmoothListBox,
    /// the nested windows, byte for byte.
    Subwindow,
}

/// The template used to build the object returned by `window`'s typed lookup
/// `find` for `name`; callers bind values to it through [`Bindings::bind`].
/// `None` means no object was found. Arrays return the first case-folded name,
/// maps the last, and each object is built from the last child under that name
/// in the window's name tree, regardless of kind. Thus a button lookup can
/// return an object built from a `multiSpriteButtonType` with the same name.
/// Neither [`Element::find`] nor container lookups use this rule.
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

/// Refusals that end the game while an object is built, whether or not it is ever attached; what
/// only its drawing needs (a resource, an input, a pipeline) does not matter for one never drawn.
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

/// Lay out a fixed-window root from its supplied top-left position. Retain
/// its background-derived content size, child construction order and inherited
/// clip. The lifecycle instance dispatches this layout.
pub fn fixed_window_layout(
    window: &Element,
    resources: &dyn Resources,
    bindings: &Bindings,
) -> Result<Option<Layout>, Refusals> {
    fields(window).map_err(|refusal| vec![refusal])?;
    let gui_type = |name: &[u8]| resources.gui_type(name);
    let mut reader = reader_refusals(window);
    // A background the `parent` template holds is read with it.
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
    let Some(mut drawn) = walk(None, Vec::new())? else {
        return Ok(None);
    };
    if let Some(pointer) = &bindings.pointer {
        let (hovered, boxes) = pointer_targets(&drawn.parts, &drawn.hits, pointer)?;
        if hovered.is_some() || !boxes.is_empty() || pointer.pressed.is_some() {
            drawn = walk(hovered, boxes)?.expect("drawn in the first pass");
        }
    }
    // A background drawable supplies the list row's content size; without a
    // background use the window size.
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
    /// Place `window` with its top-left at `origin`. Draw when `draw` is true;
    /// otherwise build it without drawing.
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
                scroll: None,
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
        // Attach the window-sized background first, then the declared child objects.
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
            // Its own object, whether its type is one of the window's children (which may be
            // built again as one) or its `parent` template's ([`Role::Background`]).
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
        // The attach loop over the order list.
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
        // Hidden or unattached objects draw nothing, but their construction
        // failures still apply.
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
            // One logical-viewport C2dPositionObject, flag144=1, unlike a
            // container's separate outer and scaled content objects
            //
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

    /// Draw one built window object at `origin` with `extent`, the size every factory
    /// receives. With `construct`, only build it to apply its construction effects.
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
            // Build a standard button with orientation 0, no extent, size or border,
            // scale 1, no rotation and frame 1, using the type's sprite, position and font.
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
            // Hide the constructed object before its later lifecycle writes.
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
                // built against this window's size.
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

    /// Bake a text box through the sprite registered under its own name. A matching
    /// text-sprite type supplies its effect; another sprite kind drops the text.
    /// Text blocks retain their layout and alignment rules.
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
        // No replayed mission-entry path sets this text box's text; construction
        // retains the authored text.
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

/// Look up one order-list `name` for attachment in this priority: scrollbars,
/// buttons, multi-sprite buttons, text boxes, instant text, edit boxes, icons,
/// browsers, curve graphs, check boxes, standard and smooth list boxes, nested
/// windows, then overlapping lists. Drop-downs are never attached. Arrays give
/// the first case-folded name; maps give the last.
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
