
use std::collections::{BTreeMap, BTreeSet};

use super::{
    Cause, Element, Fields, Gui, Kind, Orientation, Refusal, SCROLLBAR_LISTED, Scrollbar, Window,
    WindowExtent,
};

#[derive(Clone, Copy)]
enum Scope<'a> {
    Global(&'a BTreeMap<Vec<u8>, (usize, usize)>, &'a [&'a mut Gui]),
    Children(&'a [Element], bool),
}

impl<'a> Scope<'a> {
    fn find(&self, name: &[u8]) -> Option<&'a Element> {
        match *self {
            Scope::Global(map, files) => map
                .get(&name.to_ascii_lowercase())
                .map(|(file, index)| &files[*file].templates[*index]),
            Scope::Children(children, last) => {
                let mut matching = children.iter().filter(|child| child.is_named(name));
                if last {
                    matching.next_back()
                } else {
                    matching.next()
                }
            }
        }
    }
}

enum Edit {
    Sprite(Vec<u8>),
    Orientation(Orientation),
    Scrollbar(Box<Scrollbar>),
    Window(Box<Window>),
    Refuse(&'static str, String),
    Unrecorded(&'static str, String),
}

type Place = (usize, usize, Vec<usize>);

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Comparisons([BTreeMap<i32, bool>; 2]);

impl Comparisons {
    pub(super) fn matches(&self, viewport: [i32; 2]) -> bool {
        self.0.iter().zip(viewport).all(|(tested, size)| {
            tested.iter().all(|(&less, &zero)| (size == less) == zero)
        })
    }
}

pub(super) struct Viewport {
    pub(super) size: Option<[i32; 2]>,
    pub(super) comparisons: Comparisons,
    pub(super) needed: bool,
}

impl Viewport {
    pub(super) fn new(size: Option<[i32; 2]>) -> Self {
        Self { size, comparisons: Comparisons::default(), needed: false }
    }
}

pub(super) fn pass(files: &mut [&mut Gui], viewport: &mut Viewport) {
    let mut global: BTreeMap<Vec<u8>, (usize, usize)> = BTreeMap::new();
    for (file, gui) in files.iter().enumerate() {
        for (name, index) in &gui.registered {
            global.insert(name.clone(), (file, *index));
        }
    }
    let mut edits: Vec<(Place, Edit)> = Vec::new();
    {
        let shared: &[&mut Gui] = files;
        for &(file, index) in global.values() {
            let template = &shared[file].templates[index];
            visit(
                template,
                Scope::Global(&global, shared),
                (file, index, Vec::new()),
                &mut edits,
            );
        }
        windows(&global, shared, viewport, &mut edits);
    }
    for ((file, index, path), edit) in edits {
        let mut element = &mut files[file].templates[index];
        for step in path {
            element = element
                .nested_mut()
                .into_iter()
                .nth(step)
                .expect("a place the pass took from the same tree");
        }
        apply(element, edit);
    }
}

fn apply(element: &mut Element, edit: Edit) {
    let is_unrecorded = matches!(edit, Edit::Unrecorded(..));
    match edit {
        Edit::Sprite(sprite) => {
            if let Fields::Button(button) = &mut element.fields {
                button.sprite = sprite;
            }
        }
        Edit::Orientation(orientation) => {
            if let Fields::Button(button) = &mut element.fields {
                button.orientation = Some(orientation);
            }
        }
        Edit::Scrollbar(scrollbar) => {
            if let Fields::Scrollbar(own) = &mut element.fields {
                *own = *scrollbar;
            }
        }
        Edit::Window(window) => {
            if let Fields::Window(own) = &mut element.fields {
                *own = *window;
            }
        }
        Edit::Refuse(key, reason) | Edit::Unrecorded(key, reason) => {
            let cause = if is_unrecorded {
                Cause::Unrecorded
            } else {
                Cause::Unread
            };
            let refusal = Refusal {
                path: element.path.clone(),
                key: key.to_owned(),
                reason,
                cause,
                node: element.node,
            };
            if !element.refusals.contains(&refusal) {
                element.refusals.push(refusal);
            }
        }
    }
}

fn visit<'a>(element: &'a Element, scope: Scope<'a>, place: Place, edits: &mut Vec<(Place, Edit)>) {
    let mut stack = vec![(element, scope, place)];
    while let Some((element, scope, place)) = stack.pop() {
        let nested = element.nested();
        let child = |index: usize, place: &Place| {
            let mut path = place.2.clone();
            path.push(index);
            (place.0, place.1, path)
        };
        let offset = nested.len() - element.children.len();
        match &element.fields {
            Fields::Button(_) => button(element, scope, &place, edits),
            Fields::Scrollbar(own) => {
                for (index, kid) in element.children.iter().enumerate() {
                    stack.push((
                        kid,
                        Scope::Children(&element.children, true),
                        child(offset + index, &place),
                    ));
                }
                scrollbar(element, own, scope, &place, edits);
            }
            Fields::ListBox(list) if !list.parent.is_empty() => edits.push((
                place.clone(),
                Edit::Refuse(
                    "parent",
                    "is set, and list box post-load pass reads `step` as unset forever, so its \
                     walk ends only in a null read or never: the game crashes or hangs"
                        .into(),
                ),
            )),
            Fields::Container(_) => {
                for (index, kid) in element.children.iter().enumerate() {
                    stack.push((
                        kid,
                        Scope::Children(&element.children, false),
                        child(offset + index, &place),
                    ));
                }
            }
            Fields::Window(_) => {
                for (index, kid) in element.children.iter().enumerate() {
                    stack.push((
                        kid,
                        Scope::Children(&element.children, true),
                        child(offset + index, &place),
                    ));
                }
            }
            Fields::DropDown(_) | Fields::ExtendedScrollbar(_) => {
                for (index, part) in nested.iter().enumerate().take(offset) {
                    stack.push((part, scope, child(index, &place)));
                }
            }
            _ => {}
        }
    }
}

fn button(element: &Element, scope: Scope, place: &Place, edits: &mut Vec<(Place, Edit)>) {
    let Some(own) = element.button() else {
        return;
    };
    if !own.parent.is_empty() && own.sprite.is_empty() {
        match inherited_sprite(element, &own.parent, scope) {
            Ok(sprite) => edits.push((place.clone(), Edit::Sprite(sprite))),
            Err(reason) => {
                edits.push((place.clone(), Edit::Refuse("parent", reason)));
                return;
            }
        }
    }
    edits.push((
        place.clone(),
        Edit::Orientation(Orientation::parse_gui(&own.orientation_text)),
    ));
}

fn inherited_sprite(element: &Element, parent: &[u8], scope: Scope) -> Result<Vec<u8>, String> {
    let name = element.name_bytes();
    let mut seen = BTreeSet::new();
    let mut next = parent.to_vec();
    loop {
        let Some(found) = scope.find(&next) else {
            return Err(format!(
                "names {:?}, which the lookup does not find: button sprite inheritance reads the \
                 missing parent's sprite through null and the game crashes",
                String::from_utf8_lossy(&next)
            ));
        };
        let Some(fields) = found.button() else {
            return Err(format!(
                "names {}, a {}, whose button sprite inheritance reads as a button's \
                 sprite string: a type-confused read",
                found.path,
                found.kind.lookup()
            ));
        };
        if !fields.sprite.is_empty() {
            return Ok(fields.sprite.clone());
        }
        if fields.parent.is_empty() || fields.parent == name || !seen.insert(next.clone()) {
            return Err(
                "takes its sprite from a chain of parents that ends without one or turns back, \
                 which the button post-load pass walks forever: the game hangs"
                    .into(),
            );
        }
        next = fields.parent.clone();
    }
}

fn scrollbar(
    element: &Element,
    own: &Scrollbar,
    scope: Scope,
    place: &Place,
    edits: &mut Vec<(Place, Edit)>,
) {
    let listed = match usize::try_from(own.listed) {
        Ok(count) if count <= SCROLLBAR_LISTED.len() => count,
        Ok(13) if own.parent.is_empty() => SCROLLBAR_LISTED.len(),
        _ => {
            edits.push((
                place.clone(),
                Edit::Refuse(
                    "slider_pos_shift",
                    format!(
                        "wrote the listed field ids' count as {}, and scrollbar \
                         unset-field test walks that many ids from a block of thirteen (the \
                         thirteenth never written; a negative count never ends): an invalid read",
                        own.listed
                    ),
                ),
            ));
            return;
        }
    };
    if own.parent.is_empty() {
        return;
    }
    let ids = &SCROLLBAR_LISTED[..listed];
    if !own.min_value_set && ids.contains(&0x95) {
        edits.push((
            place.clone(),
            Edit::Refuse(
                "minValue",
                "is not set, and scrollbar unset-field test reads the field the constructor \
                 leaves uninitialised to decide the parent copy: an undefined read"
                    .into(),
            ),
        ));
        return;
    }
    let mut result = own.clone();
    if all_set(&result, ids) {
        return;
    }
    let name = element.name_bytes();
    let mut seen = BTreeSet::new();
    let mut next = own.parent.clone();
    loop {
        let found = match scope.find(&next) {
            Some(found) => found,
            None => {
                edits.push((
                    place.clone(),
                    Edit::Refuse(
                        "parent",
                        format!(
                            "names {:?}, which the lookup does not find: scrollbar field \
                             inheritance reads the missing parent through null and the game \
                             crashes",
                            String::from_utf8_lossy(&next)
                        ),
                    ),
                ));
                return;
            }
        };
        let Fields::Scrollbar(from) = &found.fields else {
            edits.push((
                place.clone(),
                Edit::Refuse(
                    "parent",
                    format!(
                        "names {}, a {}, which the scrollbar field inheritance reads as a \
                         scrollbar: a type-confused read",
                        found.path,
                        found.kind.lookup()
                    ),
                ),
            ));
            return;
        };
        if ids.contains(&0x95)
            && !from.min_value_set
            && result.min_value().is_some_and(|value| value == 0.0)
        {
            edits.push((
                place.clone(),
                Edit::Refuse(
                    "minValue",
                    format!(
                        "is copied from {}, whose minValue the constructor leaves uninitialised: \
                         an undefined read",
                        found.path
                    ),
                ),
            ));
            return;
        }
        copy_unset(&mut result, from, ids);
        if all_set(&result, ids) {
            edits.push((place.clone(), Edit::Scrollbar(Box::new(result))));
            return;
        }
        if from.parent.is_empty() || from.parent == name || !seen.insert(next.clone()) {
            edits.push((
                place.clone(),
                Edit::Refuse(
                    "parent",
                    "leaves a listed field unset along a chain of parents that ends or turns \
                     back, which the scrollbar post-load pass walks forever or reads through \
                     null: the game hangs or crashes"
                        .into(),
                ),
            ));
            return;
        }
        next = from.parent.clone();
    }
}

fn is_set(bar: &Scrollbar, id: u32) -> bool {
    let point = |point: [i32; 2]| point[0] != 0 && point[1] != 0;
    let float = |slot: usize| f32::from_bits(bar.slots[slot]) != 0.0;
    match id {
        0x8B => !bar.track.is_empty(),
        0x8A => !bar.slider.is_empty(),
        0x89 => !bar.right_button.is_empty(),
        0x88 => !bar.left_button.is_empty(),
        0x4C => point(bar.position),
        0x8D => float(6),
        0x2F => point(bar.size),
        0x86 => point(bar.border_size),
        0x96 => float(2),
        0x95 => float(3),
        0x97 => float(4),
        _ => float(5),
    }
}

fn all_set(bar: &Scrollbar, ids: &[u32]) -> bool {
    ids.iter().all(|id| is_set(bar, *id))
}

fn copy_unset(bar: &mut Scrollbar, from: &Scrollbar, ids: &[u32]) {
    for id in ids {
        if is_set(bar, *id) {
            continue;
        }
        match id {
            0x8B => bar.track.clone_from(&from.track),
            0x8A => bar.slider.clone_from(&from.slider),
            0x89 => bar.right_button.clone_from(&from.right_button),
            0x88 => bar.left_button.clone_from(&from.left_button),
            0x4C => bar.position = from.position,
            0x2F => bar.size = from.size,
            0x86 => bar.border_size = from.border_size,
            0x8D => bar.slots[6] = from.slots[6],
            0x96 => bar.slots[2] = from.slots[2],
            0x95 => bar.slots[3] = from.slots[3],
            0x97 => bar.slots[4] = from.slots[4],
            _ => bar.slots[5] = from.slots[5],
        }
    }
}

fn windows(
    global: &BTreeMap<Vec<u8>, (usize, usize)>,
    files: &[&mut Gui],
    viewport: &mut Viewport,
    edits: &mut Vec<(Place, Edit)>,
) {
    let mut passed: BTreeMap<*const Element, Window> = BTreeMap::new();
    for &(file, index) in global.values() {
        let template = &files[file].templates[index];
        window_pass(
            template,
            Scope::Global(global, files),
            (file, index, Vec::new()),
            viewport,
            &mut passed,
            edits,
        );
    }
}

fn window_pass<'a>(
    element: &'a Element,
    scope: Scope<'a>,
    place: Place,
    viewport: &mut Viewport,
    passed: &mut BTreeMap<*const Element, Window>,
    edits: &mut Vec<(Place, Edit)>,
) {
    if element.kind != Kind::Window {
        return;
    }
    let offset = element.nested().len() - element.children.len();
    let mut names: Vec<Vec<u8>> = element
        .children
        .iter()
        .map(|child| child.name_bytes().to_ascii_lowercase())
        .collect();
    names.sort();
    names.dedup();
    for name in names {
        let Some(index) = element
            .children
            .iter()
            .rposition(|child| child.is_named(&name))
        else {
            continue;
        };
        let mut path = place.2.clone();
        path.push(offset + index);
        window_pass(
            &element.children[index],
            Scope::Children(&element.children, true),
            (place.0, place.1, path),
            viewport,
            passed,
            edits,
        );
    }
    match window_fields(element, scope, viewport, passed) {
        Ok(Some(window)) => {
            passed.insert(element, window.clone());
            edits.push((place, Edit::Window(Box::new(window))));
        }
        Ok(None) => {}
        Err(Unfinished::Game(key, reason)) => edits.push((place, Edit::Refuse(key, reason))),
        Err(Unfinished::Unrecorded(key, reason)) => {
            edits.push((place, Edit::Unrecorded(key, reason)));
        }
    }
}

enum Unfinished {
    Game(&'static str, String),
    Unrecorded(&'static str, String),
}

fn any_unset(window: &Window, viewport: &mut Viewport) -> Result<bool, String> {
    let point = |point: [i32; 2]| point[0] == 0 || point[1] == 0;
    Ok(size_unset(window, viewport)?
        || point(window.position)
        || window.background.is_empty()
        || window.vertical_border.is_empty()
        || window.horizontal_border.is_empty())
}

fn size_unset(window: &Window, viewport: &mut Viewport) -> Result<bool, String> {
    let needs_viewport = window.size.iter().any(|extent| matches!(extent, WindowExtent::MaxResolution { .. }));
    viewport.needed |= needs_viewport;
    if needs_viewport && viewport.size.is_none() {
        return Err(
            "was read with no load-time GUI viewport given (read_layers_at, read_at): \
             window post-load pass tests whether size is set (a zero axis is unset) \
             while an axis of it is max_resolution, the GUI root's extent (graphics \
             context's render size) when the file was read less a number".into()
        );
    }
    Ok(window.size.iter().enumerate().any(|(axis, extent)| match *extent {
        WindowExtent::Pixels(value) => value == 0,
        WindowExtent::MaxResolution { less } => {
            let zero = viewport.size.expect("a max-resolution extent has a viewport")[axis] == less;
            viewport.comparisons.0[axis].insert(less, zero);
            zero
        }
    }))
}

fn window_fields(
    element: &Element,
    scope: Scope,
    viewport: &mut Viewport,
    passed: &BTreeMap<*const Element, Window>,
) -> Result<Option<Window>, Unfinished> {
    let Fields::Window(read) = &element.fields else {
        return Ok(None);
    };
    let mut own = read.clone();
    let parent = |why: String| Unfinished::Game("parent", why);
    let unrecorded = |why: String| Unfinished::Unrecorded("size", why);
    if !own.parent.is_empty() && any_unset(&own, viewport).map_err(unrecorded)? {
        let name = element.name_bytes();
        let mut seen = BTreeSet::new();
        let mut next = own.parent.clone();
        loop {
            let Some(found) = scope.find(&next) else {
                return Err(parent(format!(
                    "names {:?}, which the lookup does not find: window post-load pass copies \
                     each unset key through the null pointer it gets (window field inheritance \
                     reads the parent's size through null) and the game crashes",
                    String::from_utf8_lossy(&next)
                )));
            };
            let Fields::Window(from) = &found.fields else {
                return Err(parent(format!(
                    "names {}, a {}, whose window post-load pass reads as a \
                     windowType's size, position, background and borders: a type-confused read",
                    found.path,
                    found.kind.lookup()
                )));
            };
            let from = passed
                .get(&std::ptr::from_ref::<Element>(found))
                .unwrap_or(from)
                .clone();
            if size_unset(&own, viewport).map_err(unrecorded)? {
                own.size = from.size;
            }
            if own.position[0] == 0 || own.position[1] == 0 {
                own.position = from.position;
            }
            for (field, from) in [
                (&mut own.background, &from.background),
                (&mut own.vertical_border, &from.vertical_border),
                (&mut own.horizontal_border, &from.horizontal_border),
            ] {
                if field.is_empty() {
                    field.clone_from(from);
                }
            }
            if !any_unset(&own, viewport).map_err(unrecorded)? {
                break;
            }
            if from.parent == name || !seen.insert(next.to_ascii_lowercase()) {
                return Err(parent(
                    "leaves a listed key unset along a chain of parents that turns back, which \
                     the window post-load pass walks forever: the game hangs"
                        .into(),
                ));
            }
            next = from.parent;
        }
    }
    if own.full_screen {
        own.size = [WindowExtent::MaxResolution { less: 0 }; 2];
    }
    Ok((own != *read).then_some(own))
}
