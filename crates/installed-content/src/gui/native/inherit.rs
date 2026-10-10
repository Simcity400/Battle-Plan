//! The post-load pass: each registered template visits its children using the parent's name
//! tree as the lookup.
//!
//! Containers pass their own tree (`+0x2A0`), retaining the first child of each lowered name.
//! Backgrounds are outside that tree, so the pass does not reach them. Windows and scrollbars
//! pass their own trees (`+0x220`), retaining the last child of each name. Drop-downs and
//! extended scrollbars pass on the incoming tree, while a drop-down's inner container passes
//! its own. All five extended scrollbar parts, including its background (`+0x198`), receive the
//! incoming tree. Grids do nothing in this pass. Templates use the global registry for parent
//! lookup.
//!
//! Buttons with a set `parent` and empty sprite copy the parent's sprite without a null check.
//! While still empty, they follow that parent's `parent` when set and different from the
//! button's own name; otherwise the loop repeats forever. The orientation text is parsed
//! afterwards. Scrollbars with a set `parent` copy unset fields among their twelve listed
//! fields: empty names, points with a zero coordinate and zero floats. They continue up the
//! parent chain until all are set; a chain that cannot finish hangs or reads through null. List
//! boxes always treat `step` as unset, so a set `parent` ends in a null read or an endless
//! walk. Windows visit their children first. With a set `parent`, they copy any unset field
//! among the five listed at `+0x260`, continuing up the chain while any stay unset. A
//! `fullScreen` window then takes the GUI root's extent as its size. Pass order matters: a
//! previously visited parent supplies its pass result, while an unvisited parent supplies the
//! values it read ([`windows`]).

use std::collections::{BTreeMap, BTreeSet};

use super::{
    Cause, Element, Fields, Gui, Kind, Orientation, Refusal, SCROLLBAR_LISTED, Scrollbar, Window,
    WindowExtent,
};

/// Where a post-load pass looks a `parent` up.
#[derive(Clone, Copy)]
enum Scope<'a> {
    /// The global registry: each lowered name's (file, template).
    Global(&'a BTreeMap<Vec<u8>, (usize, usize)>, &'a [&'a mut Gui]),
    /// A parent's own tree over its children: the first of a name, or the last.
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

/// What the pass writes into one element.
enum Edit {
    Sprite(Vec<u8>),
    Orientation(Orientation),
    Scrollbar(Box<Scrollbar>),
    Window(Box<Window>),
    Refuse(&'static str, String),
    /// A refusal for a value of the recording client's session the recording does not hold.
    Unrecorded(&'static str, String),
}

/// An element by its place: the file, the template, then indices into each level's
/// [`Element::nested`].
type Place = (usize, usize, Vec<usize>);

/// Run every registered template's post-load pass over `files`, with later templates of a name
/// replacing earlier ones. `viewport` is the reading client's GUI root extent at load
/// (`+0x180`), stored less the authored number by a `max_resolution` size.
pub(super) fn pass(files: &mut [&mut Gui], viewport: Option<[i32; 2]>) {
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

/// One element's post-load pass with `scope`, and its children's, on a stack of its own.
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
            // Their parts (the extended scrollbar's background included) with the lookup they
            // were given.
            Fields::DropDown(_) | Fields::ExtendedScrollbar(_) => {
                for (index, part) in nested.iter().enumerate().take(offset) {
                    stack.push((part, scope, child(index, &place)));
                }
            }
            _ => {}
        }
    }
}

/// One button's post-load pass.
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

/// The sprite a button's `parent` chain gives it, or what the game does instead.
fn inherited_sprite(element: &Element, parent: &[u8], scope: Scope) -> Result<Vec<u8>, String> {
    let name = element.name_bytes();
    let mut seen = BTreeSet::new();
    let mut next = parent.to_vec();
    loop {
        let Some(found) = scope.find(&next) else {
            return Err(format!(
                "names {:?}, which the lookup does not find: button sprite inheritance reads the \
                 missing parent through null ([0 + 0x1B0]) and the game crashes",
                String::from_utf8_lossy(&next)
            ));
        };
        let Some(fields) = found.button() else {
            return Err(format!(
                "names {}, a {}, whose +0x1B0 button sprite inheritance reads as a button's \
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

/// Copy a scrollbar's unset fields from its `parent` chain.
fn scrollbar(
    element: &Element,
    own: &Scrollbar,
    scope: Scope,
    place: &Place,
    edits: &mut Vec<(Place, Edit)>,
) {
    // Inspect `listed` ids from `+0x268` on every pass, even without a parent. A negative count
    // never reaches the end pointer; a count past thirteen reads beyond the block. The
    // thirteenth slot is unwritten and decides copying only when a parent is set.
    let listed = match usize::try_from(own.listed) {
        Ok(count) if count <= SCROLLBAR_LISTED.len() => count,
        Ok(13) if own.parent.is_empty() => SCROLLBAR_LISTED.len(),
        _ => {
            edits.push((
                place.clone(),
                Edit::Refuse(
                    "slider_pos_shift",
                    format!(
                        "wrote the listed field ids' count (+0x274) as {}, and scrollbar \
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
                 leaves uninitialised (+0x1FC) to decide the parent copy: an undefined read"
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
                        "is copied from {}, whose minValue the constructor leaves uninitialised \
                         (+0x1FC): an undefined read",
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

/// A listed field is unset when its name is empty, its point has a zero coordinate or its float
/// is zero.
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

/// Whether all listed fields are set.
fn all_set(bar: &Scrollbar, ids: &[u32]) -> bool {
    ids.iter().all(|id| is_set(bar, *id))
}

/// Copy each listed field found unset from `from`.
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

/// The window part of the post-load pass, in encounter order: registry templates in name tree
/// order, then each window's children first and the window itself. Its child tree (`+0x220`)
/// visits each lowered name once in order, selecting the last child. A parent supplies its
/// values at the time of copying: its pass result if already visited, otherwise the values it
/// read.
fn windows(
    global: &BTreeMap<Vec<u8>, (usize, usize)>,
    files: &[&mut Gui],
    viewport: Option<[i32; 2]>,
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

/// One window's children's passes, then its own ([`window_fields`]).
fn window_pass<'a>(
    element: &'a Element,
    scope: Scope<'a>,
    place: Place,
    viewport: Option<[i32; 2]>,
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

/// Why a window's own pass is refused: the game cannot finish it, or it depends on what the
/// recording does not hold.
enum Unfinished {
    Game(&'static str, String),
    Unrecorded(&'static str, String),
}

/// Test the five listed window keys at `+0x260`: `size` 0x2F, `position` 0x4C, `background`
/// 0x9C, `verticalBorder` 0xA7 and `horizontalBorder` 0xA6. A point with a zero coordinate or
/// an empty name is unset. A `max_resolution` axis stores the reading client's root extent less
/// its number as an `i32`; return `Err` if the caller supplied no `viewport`.
fn any_unset(window: &Window, viewport: Option<[i32; 2]>) -> Result<bool, String> {
    let size = stored_size(window, viewport)?;
    let point = |point: [i32; 2]| point[0] == 0 || point[1] == 0;
    Ok(point(size)
        || point(window.position)
        || window.background.is_empty()
        || window.vertical_border.is_empty()
        || window.horizontal_border.is_empty())
}

/// The size `+0x158` holds as the game stored it at load: a `max_resolution` axis the GUI root's
/// extent less its number (window size reader, `i32`).
fn stored_size(window: &Window, viewport: Option<[i32; 2]>) -> Result<[i32; 2], String> {
    let mut size = [0i32; 2];
    for (axis, extent) in window.size.iter().enumerate() {
        match *extent {
            WindowExtent::Pixels(value) => size[axis] = value,
            WindowExtent::MaxResolution { less } => match viewport {
                Some(viewport) => size[axis] = viewport[axis].wrapping_sub(less),
                None => {
                    return Err(
                        "was read with no load-time GUI viewport given (read_layers_at, read_at): \
                         window post-load pass tests whether size is set (a zero axis is unset) \
                         while an axis of it is max_resolution, the GUI root's extent (graphics \
                         context+0x180) when the file was read less a number"
                            .into(),
                    );
                }
            },
        }
    }
    Ok(size)
}

/// The window's own post-load pass: the resulting fields, with `Ok(None)` when unchanged, or
/// the refused key and why the game cannot finish.
fn window_fields(
    element: &Element,
    scope: Scope,
    viewport: Option<[i32; 2]>,
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
                     reads [0 + 0x158]) and the game crashes",
                    String::from_utf8_lossy(&next)
                )));
            };
            let Fields::Window(from) = &found.fields else {
                return Err(parent(format!(
                    "names {}, a {}, whose +0x138..+0x1C0 window post-load pass reads as a \
                     windowType's size, position, background and borders: a type-confused read",
                    found.path,
                    found.kind.lookup()
                )));
            };
            let from = passed
                .get(&(found as *const Element))
                .unwrap_or(from)
                .clone();
            // The window field inheritance copies each unset key's whole field (`size` both axes, 8 bytes).
            let size_unset = stored_size(&own, viewport)
                .map_err(unrecorded)?
                .contains(&0);
            if size_unset {
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
            // The parent's `parent` (its vtable `+0x58`) against the window's own name (`+0x48`),
            // byte for byte: equal, the walk keeps the same parent, which copies nothing new, and
            // never ends; a chain that comes back round does the same.
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
    // A `fullScreen` window (`+0x410`) takes the GUI root extent (`+0x210`, then `+0x180`) as
    // its size.
    if own.full_screen {
        own.size = [WindowExtent::MaxResolution { less: 0 }; 2];
    }
    Ok((own != *read).then_some(own))
}
