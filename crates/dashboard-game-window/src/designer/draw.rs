//! Draws a [`Designer`] from content alone: constructs the root and its cells, makes the calls
//! the designer's refresh makes, and lays them out.

use std::{collections::BTreeMap, io};

use installed_content_core::{
    ResolvedLayers,
    gui::native::{Element, Fields, Kind},
};
use installed_content_localisation::localisation::Localisation;

use super::*;
use crate::scene::Counted;

/// The setter the designer's sprite assignment takes for `element`: a button keeps its button
/// placement, anything else is placed as an icon.
fn setter(element: &Element) -> SpriteSetter {
    match element.kind {
        Kind::Button | Kind::GuiButton | Kind::Checkbox | Kind::MultiSpriteButton => {
            SpriteSetter::Button
        }
        _ => SpriteSetter::Icon,
    }
}

fn slot(name: &str, state: Slot, detail: Option<String>) -> WindowSlot {
    WindowSlot {
        name: name.to_owned(),
        state,
        detail,
    }
}

/// A slot filled unless something it shows was refused, the reasons counted.
fn counted(name: &str, refused: Counted, unbound: Option<String>) -> WindowSlot {
    let lines = refused.lines();
    if !lines.is_empty() {
        slot(name, Slot::Refused, Some(lines.join("; ")))
    } else if let Some(why) = unbound {
        slot(name, Slot::Unbound, Some(why))
    } else {
        slot(name, Slot::Bound, None)
    }
}

/// The template name's edit box: the `name` child of the header's `divisions` drop-down, whose
/// children belong to its inner container.
fn name_box<'a>(root: &'a Element, header: &str) -> Option<&'a Element> {
    let divisions = root.path_named(&format!("{header}/divisions"))?;
    match &divisions.fields {
        Fields::DropDown(drop_down) => drop_down.inner.path_named(NAME_BOX),
        _ => divisions.path_named(NAME_BOX),
    }
}

/// The calls one composition cell's refresh makes on its entry, in the designer's order:
/// initial visibility, then the resting state.
fn cell(
    instance: &mut Instance,
    entry: &Element,
    kind: GridKind,
    drawn: &DrawnCell,
    declared: &dyn Fn(&[u8]) -> bool,
    nato: bool,
    glyphs: &mut Counted,
) -> Result<(), String> {
    let control = |path| compose::control(entry, path);
    let (unit, empty, name) = (
        control(CELL_UNIT)?,
        control(CELL_EMPTY)?,
        control(CELL_NAME)?,
    );
    for hidden in [unit, empty, name] {
        instance.hide(entry, hidden)?;
    }
    // The construction leaves these hidden; neither is drawn at rest.
    for path in [CELL_GLOW, CELL_HQ_REMOVE] {
        if let Some(hidden) = entry.path_named(path) {
            instance.hide(entry, hidden)?;
        }
    }
    if kind == GridKind::Regiments {
        let background = control(CELL_BACKGROUND)?;
        instance.set_sprite(entry, background, REGIMENT_BACKGROUND, setter(background))?;
    }
    let real = drawn.definition.is_some();
    if real {
        instance.show(entry, unit)?;
    } else {
        instance.show(entry, empty)?;
        instance.show(entry, name)?;
    }
    instance.hide(entry, name)?;
    match drawn.cell.state {
        SlotState::Filled => match &drawn.definition {
            // A hole in the filled region keeps the empty control as constructed: the glyph
            // selector refuses the null definition.
            None => instance.hide(entry, unit)?,
            Some(Ok(definition)) => {
                let glyph = unit_glyph(definition);
                if declared(&glyph) {
                    instance.set_sprite(entry, unit, &glyph, setter(unit))?;
                    instance.set_frame(entry, unit, glyph_frame(nato))?;
                } else {
                    glyphs.note(format!(
                        "{} is not declared",
                        String::from_utf8_lossy(&glyph)
                    ));
                    instance.hide(entry, unit)?;
                }
            }
            Some(Err(why)) => {
                glyphs.note(why.clone());
                instance.hide(entry, unit)?;
            }
        },
        SlotState::Addable => {
            instance.hide(entry, unit)?;
            instance.show(entry, empty)?;
            instance.set_sprite(entry, empty, NEW_SLOT, setter(empty))?;
        }
        SlotState::Locked => {
            instance.hide(entry, empty)?;
            instance.show(entry, unit)?;
            instance.set_sprite(entry, unit, LOCKED_SLOT, setter(unit))?;
            instance.set_frame(entry, unit, 1)?;
        }
        SlotState::Blocked => {
            instance.hide(entry, empty)?;
            instance.show(entry, unit)?;
            let sprite = if kind == GridKind::RegimentalSupport {
                LOCKED_SLOT
            } else {
                BLOCKED_SLOT
            };
            instance.set_sprite(entry, unit, sprite, setter(unit))?;
        }
        SlotState::Detached => {}
    }
    Ok(())
}

/// The designer drawing `designer`, from content alone.
pub fn draw(
    session: &mut Session,
    layers: &ResolvedLayers,
    localisation: &Localisation,
    designer: &Designer,
    nato: bool,
    scroll: &crate::compose::Scroll,
) -> io::Result<Bound> {
    let refuse = |why: String| Bound {
        title: None,
        read_hour: None,
        scene: Err(vec![format!("{ROOT}: {why}")]),
        slots: Vec::new(),
        warnings: Vec::new(),
    };
    let Some(root) = session.guis.template(ROOT).cloned() else {
        return Ok(refuse(format!("the content declares no {ROOT} window")));
    };
    let (header, inactive) = if designer.hq {
        (HQ_HEADER, ORDINARY_HEADER)
    } else {
        (ORDINARY_HEADER, HQ_HEADER)
    };
    if root.path_named(header).is_none() {
        return Ok(refuse(format!("{ROOT} has no {header} header")));
    }
    let entries: BTreeMap<&str, Element> = [REGIMENT_ENTRY, SUPPORT_ENTRY, STAT_ENTRY]
        .into_iter()
        .filter_map(|name| Some((name, session.guis.template(name)?.clone())))
        .collect();
    // What the window binds: its templates' own artwork, the cells' glyphs and slot sprites,
    // the symbol's candidates and the texts it writes.
    let mut assets = Assets::default();
    {
        let guis = &session.guis;
        let gui_type = |name: &[u8]| guis.template(name);
        assets.extend(assets_in(&root, &gui_type, localisation));
        for entry in entries.values() {
            assets.extend(assets_in(entry, &gui_type, localisation));
        }
    }
    for sprite in [
        REGIMENT_BACKGROUND,
        NEW_SLOT,
        LOCKED_SLOT,
        BLOCKED_SLOT,
        EQUIPMENT_BUTTON,
        EQUIPMENT_BUTTON_RESTRICTED,
    ] {
        assets.sprites.insert(sprite.to_vec());
    }
    for grid in designer.grids.iter().flatten() {
        for drawn in &grid.cells {
            if let Some(Ok(definition)) = &drawn.definition {
                assets.sprites.insert(unit_glyph(definition));
            }
        }
    }
    if let Ok(symbol) = &designer.symbol {
        assets
            .sprites
            .extend(symbol.candidates.iter().map(|(sprite, _)| sprite.clone()));
    }
    for text in [&designer.name, &designer.experience].into_iter().flatten() {
        assets.add_text(text);
    }
    for row in &designer.stats {
        for text in [&row.label, &row.value].into_iter().flatten() {
            assets.add_text(text);
        }
    }
    for text in designer
        .summary
        .0
        .iter()
        .chain([&designer.summary.1])
        .flatten()
    {
        assets.add_text(text);
    }
    session.load(layers, &assets)?;
    let resources = WindowResources {
        gui: &session.gui,
        guis: &session.guis,
        localisation,
    };
    let declared = |name: &[u8]| session.gui.registry().sprite(name).is_some();
    let viewport = session.viewport;
    let mut instance = match compose::construct(&root, viewport, &resources) {
        Ok(instance) => instance,
        Err(why) => return Ok(refuse(why)),
    };
    let mut slots = Vec::new();
    let mut controls = Counted::default();
    // A control the binding names that the content lacks is refused by path; the rest of the
    // window still draws.
    let find = |path: &str, controls: &mut Counted| {
        let found = root.path_named(path);
        if found.is_none() {
            controls.note(format!("{ROOT} has no {path} control"));
        }
        found
    };
    let apply = |result: Result<(), String>, controls: &mut Counted| {
        if let Err(why) = result {
            controls.note(why);
        }
    };
    // The inactive header, and the army headquarters' own controls in an ordinary template.
    let mut hidden = vec![inactive.to_owned()];
    if !designer.hq {
        hidden.extend(HQ_ONLY.map(str::to_owned));
    }
    hidden.extend(POPUPS.map(str::to_owned));
    hidden.push(SAVE_AND_DEPLOY.to_owned());
    hidden.push(COLONIAL_FLAG.to_owned());
    for path in &hidden {
        if let Some(element) = root.path_named(path) {
            apply(instance.hide(&root, element), &mut controls);
        }
    }
    // Summary values and width: a value not shown leaves its placeholder hidden.
    let mut summary = Counted::default();
    for (path, value) in SUMMARY_TEXTS
        .iter()
        .zip(&designer.summary.0)
        .chain([(&WIDTH_TEXT, &designer.summary.1)])
    {
        let Some(element) = find(path, &mut controls) else {
            continue;
        };
        match value {
            Ok(text) => apply(
                instance.set_text(&root, element, text, &resources),
                &mut controls,
            ),
            Err(why) => {
                summary.note(why.clone());
                apply(instance.hide(&root, element), &mut controls);
            }
        }
    }
    slots.push(counted("summary stats and combat width", summary, None));
    slots.push(slot(
        "colonial force flag",
        Slot::Unbound,
        Some("hidden: when the designer shows it is not established, and the window draws no country flags".into()),
    ));
    for path in PRODUCTION_FOOTER {
        if let Some(element) = root.path_named(path) {
            apply(instance.hide(&root, element), &mut controls);
        }
    }
    slots.push(slot(
        "production cost and resources",
        Slot::Unbound,
        Some("hidden: the footer's selector endpoints are not bound yet".into()),
    ));
    // Name.
    let mut names = Counted::default();
    let name = name_box(&root, header);
    if name.is_none() {
        controls.note(format!(
            "{ROOT} has no {header}/divisions/{NAME_BOX} control"
        ));
    }
    if let Some(element) = name {
        match &designer.name {
            Ok(name) => apply(
                instance.set_text(&root, element, name, &resources),
                &mut controls,
            ),
            Err(why) => {
                names.note(why.clone());
                apply(instance.hide(&root, element), &mut controls);
            }
        }
    }
    slots.push(counted("template name", names, None));
    // Symbol: the first declared candidate, else the designer hides the button.
    let mut symbols = Counted::default();
    if let Some(element) = find(&format!("{header}/div_templ_symbol_button"), &mut controls) {
        match &designer.symbol {
            Ok(symbol) => match symbol
                .candidates
                .iter()
                .find(|(sprite, _)| declared(sprite))
            {
                Some((sprite, frame)) => {
                    apply(
                        instance.set_sprite(&root, element, sprite, setter(element)),
                        &mut controls,
                    );
                    apply(instance.set_frame(&root, element, *frame), &mut controls);
                    apply(instance.show(&root, element), &mut controls);
                }
                None => apply(instance.hide(&root, element), &mut controls),
            },
            Err(why) => {
                symbols.note(why.clone());
                apply(instance.hide(&root, element), &mut controls);
            }
        }
    }
    slots.push(counted("template symbol", symbols, None));
    // Priority.
    let mut priorities = Counted::default();
    match designer.priority {
        Ok((priority, stages)) => {
            for (button, frame) in priority_frames(priority, stages).into_iter().enumerate() {
                let path = format!("{header}/priority_button_{}", button + 1);
                match root.path_named(&path) {
                    Some(element) => {
                        apply(instance.set_frame(&root, element, frame), &mut controls)
                    }
                    None => priorities.note(format!(
                        "{ROOT} has no {path} control for priority stage {}",
                        button + 1
                    )),
                }
            }
            if let Some(element) = find(&format!("{header}/division_priority_icon"), &mut controls)
            {
                apply(
                    instance.set_frame(&root, element, priority + 1),
                    &mut controls,
                );
            }
        }
        Err(ref why) => {
            priorities.note(why.clone());
            // Unread, no priority control keeps the frames it was constructed with, whatever
            // numbers a mod gives them.
            let numbered = |element: &&Element| {
                let name = element.name_bytes();
                let prefix = b"priority_button_";
                name.len() > prefix.len()
                    && name[..prefix.len()].eq_ignore_ascii_case(prefix)
                    && name[prefix.len()..].iter().all(u8::is_ascii_digit)
            };
            for element in root
                .path_named(header)
                .into_iter()
                .flat_map(|header| header.children.iter().filter(numbered))
            {
                apply(instance.hide(&root, element), &mut controls);
            }
            if let Some(element) = find(&format!("{header}/division_priority_icon"), &mut controls)
            {
                apply(instance.hide(&root, element), &mut controls);
            }
        }
    }
    slots.push(counted("priority", priorities, None));
    // Capability icons show only beside a real regiment.
    let mut paradrop = Counted::default();
    let paradrop_icon = find(&format!("{header}/can_paradrop_icon"), &mut controls);
    let special_icon = find(&format!("{header}/special_forces_icon"), &mut controls);
    match (paradrop_icon, designer.regiments, &designer.paradrop) {
        (Some(element), true, Ok(every)) => {
            apply(instance.show(&root, element), &mut controls);
            apply(
                instance.set_frame(&root, element, 1 + i32::from(*every)),
                &mut controls,
            );
        }
        (Some(element), true, Err(why)) => {
            paradrop.note(why.clone());
            apply(instance.hide(&root, element), &mut controls);
        }
        (Some(element), false, _) => apply(instance.hide(&root, element), &mut controls),
        (None, ..) => {}
    }
    slots.push(counted("paradrop", paradrop, None));
    if let Some(element) = special_icon {
        apply(instance.hide(&root, element), &mut controls);
    }
    slots.push(if designer.regiments {
        slot(
            "special forces",
            Slot::Unbound,
            Some("hidden: the country's effective special-forces count is not recorded".into()),
        )
    } else {
        slot("special forces", Slot::Bound, None)
    });
    // Equipment settings button and warning.
    let mut equipment = Counted::default();
    if let Some(element) = find(&format!("{header}/division_equipment_btn"), &mut controls) {
        match designer.equipment_button {
            Ok(sprite) => {
                apply(
                    instance.set_sprite(&root, element, sprite, setter(element)),
                    &mut controls,
                );
                // The game enables it only while the regiment grid holds a real definition.
                match instance.control(&root, element) {
                    Ok(control) => control.enabled = Some(designer.regiments),
                    Err(why) => controls.note(why),
                }
            }
            Err(ref why) => {
                equipment.note(why.clone());
                // Its sprite tells whether the template restricts equipment.
                apply(instance.hide(&root, element), &mut controls);
            }
        }
    }
    slots.push(counted("equipment settings", equipment, None));
    if let Some(element) = root.path_named(&format!("{header}/equipment_warning")) {
        apply(instance.hide(&root, element), &mut controls);
    }
    slots.push(slot(
        "equipment warning",
        Slot::Unbound,
        Some("hidden: which designs the country could build for each required equipment type is not established from the recording".into()),
    ));
    // Army experience.
    let mut experience = Counted::default();
    if let Some(element) = find(EXPERIENCE, &mut controls) {
        match &designer.experience {
            Ok(text) => apply(
                instance.set_text(&root, element, text, &resources),
                &mut controls,
            ),
            Err(why) => {
                experience.note(why.clone());
                apply(instance.hide(&root, element), &mut controls);
            }
        }
    }
    slots.push(counted("army experience", experience, None));
    // The editor's buttons, as the game draws them over a template with no pending edit. They
    // do not act here; the app says so on hover.
    let mut hovers = Vec::new();
    for (path, enabled) in EDITOR_BUTTONS {
        let Some(element) = root.path_named(path) else {
            continue;
        };
        let applied = match enabled {
            Some(enabled) => instance
                .control(&root, element)
                .map(|control| control.enabled = Some(enabled)),
            None => instance.hide(&root, element),
        };
        apply(applied, &mut controls);
        if enabled.is_some() {
            hovers.push((
                element.path.clone(),
                compose::Hover {
                    title: "Read-only".into(),
                    details: vec!["This button does not act in the replay.".into()],
                },
            ));
        }
    }
    slots.push(slot(
        "editor",
        Slot::Unbound,
        Some(
            "read-only: reset and save are drawn disabled, as the game draws them with no pending edit; duplicate is hidden, because whether the game enables it depends on the designer's mode, which is not recorded; no pending price is shown"
                .into(),
        ),
    ));
    // Composition.
    let mut placement = Counted::default();
    let mut glyphs = Counted::default();
    let mut grids = Vec::new();
    match &designer.grids {
        Ok(drawn) => {
            for grid in drawn {
                for why in &grid.refused {
                    placement.note(format!("{} cell: {why}", grid.kind.label()));
                }
                let (Some(element), Some(entry)) = (
                    find(grid.kind.path(), &mut controls),
                    entries.get(grid.kind.entry()),
                ) else {
                    if !entries.contains_key(grid.kind.entry()) {
                        controls.note(format!(
                            "the content declares no {} window",
                            grid.kind.entry()
                        ));
                    }
                    continue;
                };
                let mut items = Vec::with_capacity(grid.cells.len());
                for drawn in &grid.cells {
                    let mut item = match compose::construct(entry, viewport, &resources) {
                        Ok(item) => item,
                        Err(why) => return Ok(refuse(why)),
                    };
                    if let Err(why) = cell(
                        &mut item,
                        entry,
                        grid.kind,
                        drawn,
                        &declared,
                        nato,
                        &mut glyphs,
                    ) {
                        return Ok(refuse(why));
                    }
                    items.push((
                        Row {
                            instance: item,
                            hover: drawn.hover.clone(),
                        },
                        drawn.cell.slot,
                    ));
                }
                grids.push(BoundGrid {
                    grid: element,
                    template: entry,
                    items,
                });
            }
        }
        Err(why) => placement.note(why.clone()),
    }
    slots.push(counted("composition", placement, None));
    // Stat rows: a value not shown keeps its row's label, with why on hover.
    let mut values = Counted::default();
    match entries.get(STAT_ENTRY) {
        Some(entry) => {
            let (label, value) = (
                compose::control(entry, STAT_LABEL),
                compose::control(entry, STAT_COUNT),
            );
            match (label, value) {
                (Ok(label_control), Ok(value_control)) => {
                    for (kind, path) in STAT_GRIDS {
                        let Some(element) = find(path, &mut controls) else {
                            continue;
                        };
                        let mut items = Vec::new();
                        for (index, row) in designer
                            .stats
                            .iter()
                            .filter(|row| row.grid == kind)
                            .enumerate()
                        {
                            let mut item = match compose::construct(entry, viewport, &resources) {
                                Ok(item) => item,
                                Err(why) => return Ok(refuse(why)),
                            };
                            let title = row
                                .label
                                .as_deref()
                                .ok()
                                .and_then(|label| dashboard_base::catalogue::plain_text(label).ok())
                                .unwrap_or_default();
                            match &row.label {
                                Ok(text) => apply(
                                    item.set_text(entry, label_control, text, &resources),
                                    &mut controls,
                                ),
                                Err(why) => {
                                    values.note(why.clone());
                                    apply(item.hide(entry, label_control), &mut controls);
                                }
                            }
                            let mut details = Vec::new();
                            match &row.value {
                                Ok(text) => {
                                    apply(
                                        item.set_text(entry, value_control, text, &resources),
                                        &mut controls,
                                    );
                                }
                                Err(why) => {
                                    values.note(why.clone());
                                    apply(item.hide(entry, value_control), &mut controls);
                                    details.push(format!("Unavailable: {why}"));
                                }
                            }
                            details.extend(designer.basis.iter().cloned());
                            let hover = Some(Hover { title, details });
                            items.push((
                                Row {
                                    instance: item,
                                    hover,
                                },
                                [0, index as i32],
                            ));
                        }
                        grids.push(BoundGrid {
                            grid: element,
                            template: entry,
                            items,
                        });
                    }
                }
                (Err(why), _) | (_, Err(why)) => controls.note(why),
            }
        }
        None => controls.note(format!("the content declares no {STAT_ENTRY} window")),
    }
    slots.push(counted("stat rows", values, None));
    slots.push(counted("unit glyphs", glyphs, None));
    slots.push(counted("controls", controls, None));
    let hovers: Vec<(&str, compose::Hover)> = hovers
        .iter()
        .map(|(path, hover)| (path.as_str(), hover.clone()))
        .collect();
    let scene = compose::compose(
        &compose::Window {
            name: ROOT,
            root: &root,
            instance: &instance,
            lists: &[],
            grids: &grids,
            hovers: &hovers,
        },
        scroll,
        &resources,
    );
    Ok(Bound {
        title: designer
            .name
            .as_ref()
            .ok()
            .and_then(|name| dashboard_base::catalogue::plain_text(name).ok()),
        read_hour: None,
        scene,
        slots,
        warnings: Vec::new(),
    })
}

/// What the content alone decides of the designer: its root, headers, entries and the controls
/// the binding names. Each refusal is one finding.
pub fn check(guis: &installed_content_core::gui::native::Guis) -> Vec<String> {
    let mut refused = Vec::new();
    let Some(root) = guis.template(ROOT) else {
        return vec![format!("the content declares no {ROOT} window")];
    };
    for header in [ORDINARY_HEADER, HQ_HEADER] {
        for path in ["", "/div_templ_symbol_button", "/division_priority_icon"] {
            let path = format!("{header}{path}");
            if root.path_named(&path).is_none() {
                refused.push(format!("{ROOT} has no {path} control"));
            }
        }
        if name_box(root, header).is_none() {
            refused.push(format!(
                "{ROOT} has no {header}/divisions/{NAME_BOX} control"
            ));
        }
    }
    for kind in GridKind::ALL {
        if root.path_named(kind.path()).is_none() {
            refused.push(format!("{ROOT} has no {} grid", kind.path()));
        }
    }
    for entry in [REGIMENT_ENTRY, SUPPORT_ENTRY] {
        match guis.template(entry) {
            Some(template) => {
                for control in [CELL_UNIT, CELL_EMPTY, CELL_NAME] {
                    if let Err(why) = compose::control(template, control) {
                        refused.push(why);
                    }
                }
            }
            None => refused.push(format!("the content declares no {entry} window")),
        }
    }
    refused
}
