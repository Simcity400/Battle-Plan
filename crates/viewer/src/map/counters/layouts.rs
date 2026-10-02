//! Counter templates selected by CUnitCounterItem::constructor@1418BE360.

use super::*;
use crate::map::native_gui;

pub(super) fn window<'a>(guis: &'a native::Guis, name: &str) -> io::Result<&'a native::Element> {
    guis.template(name)
        .filter(|element| element.kind == native::Kind::Container)
        .ok_or_else(|| {
            invalid(format!(
                "no .gui file declares {name} as a containerWindowType"
            ))
        })
}

fn supported(element: &native::Element) -> io::Result<()> {
    if let Some(refusal) = element.subtree_refusals().first() {
        return Err(invalid(format!("{}: {refusal}", element.path)));
    }
    Ok(())
}

pub(super) fn window_size(element: &native::Element, owner: &str) -> io::Result<[f32; 2]> {
    supported(element)?;
    native_gui::container_frame(element, None)
        .map(|frame| frame.size.map(|value| value as f32))
        .map_err(|why| invalid(format!("{owner}: {why}")))
}

/// Walk the kept native elements in attach order, retaining each container's content origin.
fn placement(
    root: &native::Element,
    target: &native::Element,
) -> io::Result<(u32, [f32; 2], [i32; 2])> {
    let frame = native_gui::container_frame(root, None).map_err(invalid)?;
    let mut pending: Vec<_> = root
        .children
        .iter()
        .rev()
        .map(|child| (child, frame, [0; 2]))
        .collect();
    let mut order = 0;
    while let Some((element, frame, origin)) = pending.pop() {
        let base = [
            origin[0] + frame.content_at[0],
            origin[1] + frame.content_at[1],
        ];
        if std::ptr::eq(element, target) {
            return Ok((order, base.map(|value| value as f32), frame.content_size));
        }
        order += 1;
        if element.kind == native::Kind::Container {
            let child =
                native_gui::container_frame(element, Some(frame.content_size)).map_err(invalid)?;
            // Recursive lookup only descends child containers (1422A7170).
            let at = [base[0] + child.at[0], base[1] + child.at[1]];
            pending.extend(
                element
                    .children
                    .iter()
                    .rev()
                    .map(|element| (element, child, at)),
            );
        }
    }
    Err(invalid(format!(
        "{} is not attached to {}",
        target.path, root.path
    )))
}

pub(super) fn child(
    root: &native::Element,
    name: &str,
    owner: &str,
    missing: &mut Vec<String>,
) -> io::Result<Child> {
    // BindControls@1418C70C0 uses FindIconByName's recursive kind lookup.
    let Some(element) = root.find_recursive(native::Kind::Icon, name) else {
        missing.push(format!("{owner}.{name}"));
        return Ok(Child {
            sprite: String::new(),
            at: [0.0; 2],
            scale: 1.0,
            frame: 0,
            rotation: 0.0,
            order: u32::MAX,
        });
    };
    supported(element)?;
    let icon = element.icon().expect("icon lookup");
    let (order, base, parent) = placement(root, element)?;
    let anchor = native_gui::compute_anchor(
        icon.orientation,
        native::Orientation::UpperLeft,
        parent,
        [0; 2],
    );
    Ok(Child {
        sprite: String::from_utf8(icon.sprite.clone())
            .map_err(|why| invalid(format!("{}: {why}", element.path)))?,
        at: [
            base[0] + icon.position[0] + anchor[0] as f32,
            base[1] + icon.position[1] + anchor[1] as f32,
        ],
        scale: icon.scale,
        frame: icon.frame.max(0) as u32,
        rotation: icon.rotation,
        order,
    })
}

pub(super) fn text_child(
    root: &native::Element,
    name: &str,
    owner: &str,
    missing: &mut Vec<String>,
) -> io::Result<TextChild> {
    let Some(element) = root.find_recursive(native::Kind::TextBox, name) else {
        missing.push(format!("{owner}.{name}"));
        return Ok(TextChild {
            font: None,
            order: u32::MAX,
            at: [0.0; 2],
            max_width: 0.0,
            align: text::Align::Left,
        });
    };
    supported(element)?;
    let text = element.text_box().expect("text lookup");
    let (order, base, parent) = placement(root, element)?;
    let anchor = native_gui::compute_anchor(
        text.orientation,
        native::Orientation::UpperLeft,
        parent,
        [0; 2],
    );
    Ok(TextChild {
        font: (!text.font.is_empty())
            .then(|| String::from_utf8(text.font.clone()))
            .transpose()
            .map_err(|why| invalid(format!("{}: {why}", element.path)))?,
        order,
        at: [
            base[0] + text.position[0].wrapping_add(anchor[0]) as f32,
            base[1] + text.position[1].wrapping_add(anchor[1]) as f32,
        ],
        // CInstantTextObject::ctor@1422D7A00 makes absent/zero (1422E4F40) a signed 200.
        max_width: if text.max_width == 0 {
            200.0
        } else {
            text.max_width as i32 as f32
        },
        align: match text.format {
            native::TextFormat::Left => text::Align::Left,
            native::TextFormat::Center => text::Align::Centre,
            native::TextFormat::Right => text::Align::Right,
        },
    })
}

pub(super) fn gun_counter(
    guis: &native::Guis,
    missing: &mut Vec<String>,
) -> io::Result<Option<GunCounter>> {
    let Some(gun) = guis
        .template(GUN_WINDOW)
        .filter(|element| element.kind == native::Kind::Container)
    else {
        missing.push(GUN_WINDOW.to_owned());
        return Ok(None);
    };
    Ok(Some(GunCounter {
        size: window_size(gun, GUN_WINDOW)?,
        plate: child(gun, "count_bg", GUN_WINDOW, missing)?,
        group_marker: child(gun, "group_marker", GUN_WINDOW, missing)?,
        overlay: child(gun, "counter_overlay", GUN_WINDOW, missing)?,
        no_orders: child(gun, "no_orders", GUN_WINDOW, missing)?,
        type_icon: child(gun, "type", GUN_WINDOW, missing)?,
        count: text_child(gun, "count_txt", GUN_WINDOW, missing)?,
        bar_str: child(gun, "bar_str", GUN_WINDOW, missing)?,
        flag: child(gun, "flag", GUN_WINDOW, missing)?,
        large_flag: child(gun, "large_flag", GUN_WINDOW, missing)?,
        supply: child(gun, "low_supply", GUN_WINDOW, missing)?,
        action: child(gun, "onmap_action_icon", GUN_WINDOW, missing)?,
    }))
}

pub(super) fn ability_grid(
    guis: &native::Guis,
    land: &native::Element,
    missing: &mut Vec<String>,
    unsupported: &mut Vec<String>,
) -> io::Result<Option<AbilityGrid>> {
    let Some(element) = land.find_recursive(native::Kind::Grid, "active_abilities") else {
        missing.push(format!("{LAND_WINDOW}.active_abilities"));
        return Ok(None);
    };
    let Some(item) = guis
        .template(ABILITY_WINDOW)
        .filter(|element| element.kind == native::Kind::Container)
    else {
        missing.push(ABILITY_WINDOW.to_owned());
        return Ok(None);
    };
    let Some(icon) = item
        .find_recursive(native::Kind::Icon, "active_ability")
        .and_then(native::Element::icon)
    else {
        missing.push(format!("{ABILITY_WINDOW}.active_ability"));
        return Ok(None);
    };
    supported(element)?;
    supported(item)?;
    let grid = element.grid().expect("grid lookup");
    let from_centre = icon.orientation == native::Orientation::Center;
    if grid.max_slots[1] != 1
        || native::GridFormat::from_value(grid.format) != native::GridFormat::UpperLeft
        || !grid.add_horizontal
        || !(from_centre || icon.orientation == native::Orientation::UpperLeft)
    {
        unsupported.push(format!("{LAND_WINDOW}.active_abilities: only one horizontal row from the upper left, with {ABILITY_WINDOW}.active_ability anchored at the item's upper left or centre, is drawn"));
        return Ok(None);
    }
    let (order, base, parent) = placement(land, element)?;
    let slots = native_gui::grid_slots(element, parent, 1, None).map_err(invalid)?;
    let [at] = slots.as_slice() else {
        unsupported.push(format!(
            "{}.active_abilities cannot place an item",
            LAND_WINDOW
        ));
        return Ok(None);
    };
    let (_, _, slot) = native_gui::grid_geometry(element, grid, parent).map_err(invalid)?;
    Ok(Some(AbilityGrid {
        at: [base[0] + at[0] as f32, base[1] + at[1] as f32],
        slot: slot.map(|value| value as f32),
        order,
        item_size: window_size(item, ABILITY_WINDOW)?,
        icon_at: icon.position,
        scale: icon.scale,
        from_centre,
        // CIconType::property_reader@142358910 stores this flag; origo is not an icon key.
        centred: icon.center_position,
    }))
}

pub(super) fn land_counter(
    guis: &native::Guis,
    missing: &mut Vec<String>,
) -> io::Result<LandCounter> {
    let land = window(guis, LAND_WINDOW)?;
    let hq_offset = match land
        .find(native::Kind::PositionType, "unit_counter_hq_x_offset")
        .and_then(native::Element::position_type)
    {
        Some(offset) => offset.position.map(|value| value as f32),
        None => {
            missing.push(format!("{LAND_WINDOW}.unit_counter_hq_x_offset"));
            [0.0; 2]
        }
    };
    Ok(LandCounter {
        size: window_size(land, LAND_WINDOW)?,
        plate: child(land, "count_bg", LAND_WINDOW, missing)?,
        overlay: child(land, "counter_overlay", LAND_WINDOW, missing)?,
        type_icon: child(land, "type", LAND_WINDOW, missing)?,
        bar_org: child(land, "bar_org", LAND_WINDOW, missing)?,
        bar_str: child(land, "bar_str", LAND_WINDOW, missing)?,
        supply: child(land, "low_supply", LAND_WINDOW, missing)?,
        attrition: child(land, "attrition", LAND_WINDOW, missing)?,
        digin: child(land, "onmap_digin_icon", LAND_WINDOW, missing)?,
        training: child(land, "onmap_training_icon", LAND_WINDOW, missing)?,
        no_orders: child(land, "no_orders", LAND_WINDOW, missing)?,
        defending_area: child(land, "defending_area", LAND_WINDOW, missing)?,
        invalid_orders: child(land, "invalid_orders", LAND_WINDOW, missing)?,
        flag: child(land, "flag", LAND_WINDOW, missing)?,
        large_flag: child(land, "large_flag", LAND_WINDOW, missing)?,
        count: text_child(land, "count_txt", LAND_WINDOW, missing)?,
        hq_badge: child(land, "hq_badge", LAND_WINDOW, missing)?,
        portrait: child(land, "general_portrait", LAND_WINDOW, missing)?,
        group_marker: child(land, "group_marker", LAND_WINDOW, missing)?,
        leader_enroute: child(land, "leader_enroute", LAND_WINDOW, missing)?,
        hq_offset,
    })
}

pub(super) fn column_layout(guis: &native::Guis) -> io::Result<ColumnLayout> {
    let stack = window(guis, STACK_WINDOW)?;
    stack
        .find_recursive(native::Kind::Grid, "units_box")
        .ok_or_else(|| invalid(format!("{STACK_WINDOW} has no units_box")))?;
    let offset = guis
        .template(ZOOMED_IN_OFFSET)
        .and_then(native::Element::position_type)
        .ok_or_else(|| invalid(format!("no .gui file declares {ZOOMED_IN_OFFSET}")))?;
    Ok(ColumnLayout {
        stack_size: window_size(stack, STACK_WINDOW)?,
        zoomed_in: offset.position.map(|value| value as f32),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_layout_refusals_are_independent_and_do_not_abort_loading() {
        let source = br#"guiTypes={
            containerWindowType={name=unit_counter size={width=76 height=24}}
            containerWindowType={name=unit_counter_railway_gun size={width=100% height=24}}
            containerWindowType={name=unit_counter_navy}
            containerWindowType={name=naval_combat_mapicon}
            containerWindowType={name=unit_mapicon size={width=58 height=27}
                gridBoxType={name=units_box}}
            positionType={name=unit_counter_zoomed_in_offset position={x=0 y=20}}
        }"#;
        let windows = gui_windows::NativeWindows::read(source, [1920, 1080]).unwrap();
        let layouts = Layouts::read(&windows);
        assert!(layouts.land.is_ok());
        assert!(layouts.columns.is_ok());
        assert!(layouts.gun.as_ref().unwrap_err().contains(GUN_WINDOW));
        let source = String::from_utf8_lossy(source).replace("name=units_box", "name=other_grid");
        let windows = gui_windows::NativeWindows::read(source.as_bytes(), [1920, 1080]).unwrap();
        let layouts = Layouts::read(&windows);
        assert!(layouts.land.is_ok());
        let why = layouts.columns.as_ref().unwrap_err();
        assert!(why.contains(STACK_WINDOW) && why.contains("units_box"));
    }

    #[test]
    fn counter_templates_use_the_native_registry_and_accept_eof() {
        let guis = native::Guis {
            files: vec![
                (
                    MAPICONS_GUI.into(),
                    native::read(
                        br#"guiTypes={
                containerWindowType={name=unit_counter size={width=1 height=2}}
                containerWindowType={name=unit_counter_navy}
                containerWindowType={name=naval_combat_mapicon}
                containerWindowType={name=unit_mapicon size={width=58 height=27}
                    gridBoxType={name=units_box slotsize={width=58 height=1}}}
                positionType={name=unit_counter_zoomed_in_offset position={x=0 y=20}}
            }"#,
                    ),
                ),
                (
                    "interface/mod_columns.gui".into(),
                    native::read(
                        br#"guiTypes={
                containerWindowType={name=UNIT_COUNTER size={width=82 height=28}
                    iconType={name=COUNT_BG spriteType=GFX_mod position={x=4 y=7}}
                    instantTextBoxType={name=count_txt position={x=8 y=9}}
                    instantTextBoxType={name=zero_count maxWidth=0}
                    instantTextBoxType={name=wide_count maxWidth=4294967295}
                }"#,
                    ),
                ),
            ],
        };
        let windows = gui_windows::NativeWindows::of(guis).unwrap();
        let layouts = Layouts::read(&windows);
        assert_eq!(layouts.land.as_ref().unwrap().size, [82.0, 28.0]);
        assert_eq!(layouts.land.as_ref().unwrap().plate.sprite, "GFX_mod");
        assert_eq!(layouts.land.as_ref().unwrap().plate.at, [4.0, 7.0]);
        // The absent/zero width takes ctor@1422D7A00's 200; other values keep signed bits.
        assert_eq!(layouts.land.as_ref().unwrap().count.max_width, 200.0);
        let land = window(&windows.guis, LAND_WINDOW).unwrap();
        let width = |name| {
            text_child(land, name, LAND_WINDOW, &mut Vec::new())
                .unwrap()
                .max_width
        };
        assert_eq!(width("zero_count"), 200.0);
        assert_eq!(width("wide_count"), -1.0);
        assert_eq!(
            zoom_lift(layouts.columns.as_ref().unwrap(), 120.0, 60.0),
            24.0
        );
    }

    #[test]
    fn recursive_counter_lookup_keeps_native_kind_and_parent_placement() {
        let guis = native::Guis {
            files: vec![(
                MAPICONS_GUI.into(),
                native::read(
                    br#"guiTypes={
            containerWindowType={name=unit_counter size={width=80 height=30}
                buttonType={name=flag spriteType=GFX_wrong_kind}
                containerWindowType={name=inner size={width=20 height=10} position={x=5 y=6}
                    iconType={name=FLAG spriteType=GFX_flag position={x=1 y=2}}}
                instantTextBoxType={name=count_txt font=mod_font maxWidth=17}
            }}"#,
                ),
            )],
        };
        let root = window(&guis, LAND_WINDOW).unwrap();
        let flag = child(root, "flag", LAND_WINDOW, &mut Vec::new()).unwrap();
        let count = text_child(root, "count_txt", LAND_WINDOW, &mut Vec::new()).unwrap();
        assert_eq!(count.max_width, 17.0);
        assert_eq!(flag.sprite, "GFX_flag");
        assert_eq!(flag.at, [6.0, 8.0]);
        assert!(flag.order < count.order);
    }
}
