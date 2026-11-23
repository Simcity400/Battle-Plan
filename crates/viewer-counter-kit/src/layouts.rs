
use content_reader_gui::gui::native;
use std::io;
use super::{
    ABILITY_WINDOW, AbilityGrid, Child, ColumnLayout, GUN_WINDOW, GunCounter, LAND_WINDOW,
    LandCounter, STACK_WINDOW, TextChild, ZOOMED_IN_OFFSET, invalid,
};
use viewer_native_gui::text;
#[cfg(test)]
use super::{ActionContent, Layouts, zoom_lift};
use viewer_native_gui::native_gui;
#[cfg(test)]
use viewer_native_gui::native_windows::NativeWindows;

pub fn window<'a>(guis: &'a native::Guis, name: &str) -> io::Result<&'a native::Element> {
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

pub fn window_size(element: &native::Element, owner: &str) -> io::Result<[f32; 2]> {
    supported(element)?;
    native_gui::container_frame(element, None)
        .map(|frame| frame.size.map(|value| value as f32))
        .map_err(|why| invalid(format!("{owner}: {why}")))
}

fn placement(
    root: &native::Element,
    target: &native::Element,
) -> io::Result<(u32, [f32; 2], [i32; 2])> {
    let frame = native_gui::container_frame(root, None).map_err(invalid)?;
    if let Some(order) = root
        .children
        .iter()
        .position(|element| std::ptr::eq(element, target))
    {
        return Ok((
            order as u32,
            frame.content_at.map(|value| value as f32),
            frame.content_size,
        ));
    }
    Err(invalid(format!(
        "{} is not attached to {}",
        target.path, root.path
    )))
}

pub fn child(
    root: &native::Element,
    kind: native::Kind,
    name: &str,
    owner: &str,
    missing: &mut Vec<String>,
) -> io::Result<Child> {
    let Some(element) = root.find(kind, name) else {
        missing.push(format!("{owner}.{name}"));
        return Ok(Child {
            sprite: String::new(),
            at: [0.0; 2],
            scale: 1.0,
            frame: 0,
            rotation: 0.0,
            centered: false,
            order: u32::MAX,
        });
    };
    supported(element)?;
    let (sprite, position, orientation, scale, frame, rotation) = match &element.fields {
        native::Fields::Icon(icon) => (
            &icon.sprite,
            icon.position,
            icon.orientation,
            icon.scale,
            icon.frame,
            icon.rotation,
        ),
        native::Fields::Button(button) => {
            for (unsupported, field) in [
                (button.center_position, "centerPosition"),
                (button.rotation != 0.0, "rotation"),
                (!button.enabled, "enabled=no"),
                (!button.font.is_empty(), "font"),
                (button.sprite.is_empty(), "empty sprite"),
            ] {
                if unsupported {
                    return Err(invalid(format!(
                        "{}: counter batch cannot draw button {field}",
                        element.path
                    )));
                }
            }
            (
                &button.sprite,
                button.position.map(|value| value as f32),
                button.orientation.unwrap_or_default(),
                button.scale,
                button.frame,
                button.rotation,
            )
        }
        _ => unreachable!("counter sprite lookup"),
    };
    let (order, base, parent) = placement(root, element)?;
    let anchor =
        native_gui::compute_anchor(orientation, native::Orientation::UpperLeft, parent, [0; 2]);
    Ok(Child {
        sprite: String::from_utf8(sprite.clone())
            .map_err(|why| invalid(format!("{}: {why}", element.path)))?,
        at: [
            base[0] + position[0] + anchor[0] as f32,
            base[1] + position[1] + anchor[1] as f32,
        ],
        scale,
        frame: frame.max(0) as u32,
        rotation,
        centered: element.icon().is_some_and(|icon| icon.center_position),
        order,
    })
}

pub fn text_child(
    root: &native::Element,
    name: &str,
    owner: &str,
    missing: &mut Vec<String>,
) -> io::Result<TextChild> {
    let Some(element) = root.find(native::Kind::TextBox, name) else {
        missing.push(format!("{owner}.{name}"));
        return Ok(TextChild {
            font: None,
            order: u32::MAX,
            at: [0.0; 2],
            max_width: 0.0,
            align: text::Align::Left,
            truncate: true,
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
        truncate: text.truncate,
    })
}

pub fn gun_counter(
    guis: &native::Guis,
    missing: &mut Vec<String>,
) -> io::Result<Option<GunCounter>> {
    use native::Kind::{Button, Icon};
    let Some(gun) = guis
        .template(GUN_WINDOW)
        .filter(|element| element.kind == native::Kind::Container)
    else {
        missing.push(GUN_WINDOW.to_owned());
        return Ok(None);
    };
    Ok(Some(GunCounter {
        size: window_size(gun, GUN_WINDOW)?,
        plate: child(gun, Button, "count_bg", GUN_WINDOW, missing)?,
        selected: child(gun, Icon, "count_selected", GUN_WINDOW, missing)?,
        group_marker: child(gun, Icon, "group_marker", GUN_WINDOW, missing)?,
        overlay: child(gun, Icon, "counter_overlay", GUN_WINDOW, missing)?,
        no_orders: child(gun, Icon, "no_orders", GUN_WINDOW, missing)?,
        type_icon: child(gun, Icon, "type", GUN_WINDOW, missing)?,
        count: text_child(gun, "count_txt", GUN_WINDOW, missing)?,
        bar_str: child(gun, Icon, "bar_str", GUN_WINDOW, missing)?,
        flag: child(gun, Icon, "flag", GUN_WINDOW, missing)?,
        large_flag: child(gun, Icon, "large_flag", GUN_WINDOW, missing)?,
        supply: child(gun, Icon, "low_supply", GUN_WINDOW, missing)?,
        action: child(gun, Icon, "onmap_action_icon", GUN_WINDOW, missing)?,
    }))
}

pub fn ability_grid(
    guis: &native::Guis,
    land: &native::Element,
    missing: &mut Vec<String>,
    unsupported: &mut Vec<String>,
) -> io::Result<Option<AbilityGrid>> {
    let Some(element) = land.find(native::Kind::Grid, "active_abilities") else {
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
        .find(native::Kind::Icon, "active_ability")
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
        centred: icon.center_position,
    }))
}

pub fn land_counter(
    guis: &native::Guis,
    missing: &mut Vec<String>,
) -> io::Result<LandCounter> {
    use native::Kind::{Button, Icon};
    let land = window(guis, LAND_WINDOW)?;
    let hq_offset = match land
        .find_instance("unit_counter_hq_x_offset")
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
        plate: child(land, Button, "count_bg", LAND_WINDOW, missing)?,
        selected: child(land, Icon, "count_selected", LAND_WINDOW, missing)?,
        overlay: child(land, Icon, "counter_overlay", LAND_WINDOW, missing)?,
        type_icon: child(land, Icon, "type", LAND_WINDOW, missing)?,
        bar_org: child(land, Icon, "bar_org", LAND_WINDOW, missing)?,
        bar_str: child(land, Icon, "bar_str", LAND_WINDOW, missing)?,
        supply: child(land, Icon, "low_supply", LAND_WINDOW, missing)?,
        attrition: child(land, Icon, "attrition", LAND_WINDOW, missing)?,
        digin: child(land, Icon, "onmap_digin_icon", LAND_WINDOW, missing)?,
        training: child(land, Icon, "onmap_training_icon", LAND_WINDOW, missing)?,
        action: child(land, Icon, "onmap_action_icon", LAND_WINDOW, missing)?,
        no_orders: child(land, Icon, "no_orders", LAND_WINDOW, missing)?,
        defending_area: child(land, Icon, "defending_area", LAND_WINDOW, missing)?,
        invalid_orders: child(land, Icon, "invalid_orders", LAND_WINDOW, missing)?,
        flag: child(land, Icon, "flag", LAND_WINDOW, missing)?,
        large_flag: child(land, Icon, "large_flag", LAND_WINDOW, missing)?,
        count: text_child(land, "count_txt", LAND_WINDOW, missing)?,
        hq_badge: child(land, Icon, "hq_badge", LAND_WINDOW, missing)?,
        portrait: child(land, Icon, "general_portrait", LAND_WINDOW, missing)?,
        group_marker: child(land, Icon, "group_marker", LAND_WINDOW, missing)?,
        leader_enroute: child(land, Icon, "leader_enroute", LAND_WINDOW, missing)?,
        hq_offset,
    })
}

pub fn column_layout(guis: &native::Guis) -> io::Result<ColumnLayout> {
    let stack = window(guis, STACK_WINDOW)?;
    stack
        .find(native::Kind::Grid, "units_box")
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
    use super::{
        ActionContent, GUN_WINDOW, LAND_WINDOW, Layouts, NativeWindows, STACK_WINDOW, ability_grid,
        child, column_layout, land_counter, native, text_child, window, zoom_lift,
    };
    use viewer_native_gui::native_windows::MAPICONS_GUI;

    #[test]
    fn counter_action_land_child_loads_its_authored_sprite() {
        content_reader_script::test_game::enter();
        let windows = NativeWindows::read(
            br#"guiTypes={containerWindowType={name=unit_counter_navy}
                containerWindowType={name=naval_combat_mapicon}
                containerWindowType={name=unit_counter size={width=81 height=29}
                iconType={name=onmap_action_icon spriteType=GFX_synthetic_actions
                    position={x=57 y=9}}}}"#,
            [1920, 1080],
        )
        .unwrap();
        let layouts = Layouts::read(&windows);
        assert!(layouts.sprite_names().contains(&"GFX_synthetic_actions"));
    }

    #[test]
    fn counter_action_centred_child_keeps_the_land_layout() {
        content_reader_script::test_game::enter();
        let gui = native::read(
            br#"guiTypes={containerWindowType={name=unit_counter
            iconType={name=onmap_action_icon spriteType=GFX_synthetic_actions
                centerPosition=yes}}}"#,
        );
        let guis = native::Guis::new(vec![(MAPICONS_GUI.into(), gui)]);
        let layout = land_counter(&guis, &mut Vec::new()).unwrap();
        let content = ActionContent::load(&layout.action, LAND_WINDOW, None);
        let why = content.frame(Ok(8), true, false).unwrap_err();
        assert!(why.contains("unit_counter.onmap_action_icon") && why.contains("centerPosition"));
    }

    #[test]
    fn counter_layout_refusals_are_independent_and_do_not_abort_loading() {
        content_reader_script::test_game::enter();
        let source = br#"guiTypes={
            containerWindowType={name=unit_counter size={width=76 height=24}}
            containerWindowType={name=unit_counter_railway_gun size={width=100% height=24}}
            containerWindowType={name=unit_counter_navy}
            containerWindowType={name=naval_combat_mapicon}
            containerWindowType={name=unit_mapicon size={width=58 height=27}
                gridBoxType={name=units_box}}
            positionType={name=unit_counter_zoomed_in_offset position={x=0 y=20}}
        }"#;
        let windows = NativeWindows::read(source, [1920, 1080]).unwrap();
        let layouts = Layouts::read(&windows);
        assert!(layouts.land.is_ok());
        assert!(layouts.columns.is_ok());
        assert!(layouts.gun.as_ref().unwrap_err().contains(GUN_WINDOW));
        let source = String::from_utf8_lossy(source).replace("name=units_box", "name=other_grid");
        let windows = NativeWindows::read(source.as_bytes(), [1920, 1080]).unwrap();
        let layouts = Layouts::read(&windows);
        assert!(layouts.land.is_ok());
        let why = layouts.columns.as_ref().unwrap_err();
        assert!(why.contains(STACK_WINDOW) && why.contains("units_box"));
    }

    #[test]
    fn counter_templates_use_the_native_registry_and_accept_eof() {
        content_reader_script::test_game::enter();
        let guis = native::Guis::new(vec![
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
                    buttonType={name=COUNT_BG quadTextureSprite=GFX_mod position={x=4 y=7}}
                    instantTextBoxType={name=count_txt position={x=8 y=9}}
                    instantTextBoxType={name=zero_count maxWidth=0}
                    instantTextBoxType={name=wide_count maxWidth=4294967295}
                }"#,
                ),
            ),
        ]);
        let windows = NativeWindows::of(guis).unwrap();
        let layouts = Layouts::read(&windows);
        assert_eq!(layouts.land.as_ref().unwrap().size, [82.0, 28.0]);
        assert_eq!(layouts.land.as_ref().unwrap().plate.sprite, "GFX_mod");
        assert_eq!(layouts.land.as_ref().unwrap().plate.at, [4.0, 7.0]);
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
    fn counter_lookup_keeps_native_kind_and_window_local_depth() {
        content_reader_script::test_game::enter();
        let guis = native::Guis::new(vec![(
            MAPICONS_GUI.into(),
            native::read(
                br#"guiTypes={
            containerWindowType={name=unit_counter size={width=80 height=30}
                buttonType={name=flag spriteType=GFX_wrong_kind}
                containerWindowType={name=inner size={width=20 height=10} position={x=5 y=6}
                    iconType={name=FLAG spriteType=GFX_flag position={x=1 y=2}}}
                iconType={name=large_flag spriteType=GFX_large_flag position={x=4 y=5}}
                instantTextBoxType={name=count_txt font=mod_font maxWidth=17}
                instantTextBoxType={name=plain_txt font=mod_font maxWidth=17 truncate=no}
            }}"#,
            ),
        )]);
        let root = window(&guis, LAND_WINDOW).unwrap();
        let mut missing = Vec::new();
        let flag = child(root, native::Kind::Icon, "flag", LAND_WINDOW, &mut missing).unwrap();
        let large_flag = child(
            root,
            native::Kind::Icon,
            "large_flag",
            LAND_WINDOW,
            &mut missing,
        )
        .unwrap();
        let count = text_child(root, "count_txt", LAND_WINDOW, &mut Vec::new()).unwrap();
        let plain = text_child(root, "plain_txt", LAND_WINDOW, &mut Vec::new()).unwrap();
        assert_eq!(count.max_width, 17.0);
        assert!(count.truncate);
        assert!(!plain.truncate);
        assert!(flag.sprite.is_empty());
        assert_eq!(missing, ["unit_counter.flag"]);
        assert_eq!(large_flag.sprite, "GFX_large_flag");
        assert_eq!(large_flag.at, [4.0, 5.0]);
        assert!(large_flag.order < count.order);
    }

    #[test]
    fn counter_text_grid_and_ability_lookups_are_window_local() {
        content_reader_script::test_game::enter();
        let guis = native::Guis::new(vec![(
            MAPICONS_GUI.into(),
            native::read(
                br#"guiTypes={
                containerWindowType={name=unit_counter size={width=80 height=30}
                    positionType={name=UNIT_COUNTER_HQ_X_OFFSET position={x=-20 y=0}}
                    containerWindowType={name=inner
                        instantTextBoxType={name=count_txt font=mod_font}
                        gridBoxType={name=active_abilities slotsize={width=25 height=25}
                            max_slots_vertical=1 format=UPPER_LEFT}}}
                containerWindowType={name=active_ability_item size={width=20 height=20}
                    containerWindowType={name=inner iconType={name=active_ability}}}
                containerWindowType={name=unit_mapicon size={width=58 height=27}
                    containerWindowType={name=inner gridBoxType={name=units_box}}}
                positionType={name=unit_counter_zoomed_in_offset position={x=0 y=20}}
            }"#,
            ),
        )]);
        let mut missing = Vec::new();
        let land = land_counter(&guis, &mut missing).unwrap();
        assert_eq!(land.hq_offset, [0.0; 2]);
        assert!(missing.contains(&"unit_counter.unit_counter_hq_x_offset".into()));
        assert_eq!(land.count.font, None);
        assert!(missing.contains(&"unit_counter.count_txt".into()));
        let root = window(&guis, LAND_WINDOW).unwrap();
        assert!(
            ability_grid(&guis, root, &mut missing, &mut Vec::new())
                .unwrap()
                .is_none()
        );
        assert!(missing.contains(&"unit_counter.active_abilities".into()));
        assert!(
            column_layout(&guis)
                .unwrap_err()
                .to_string()
                .contains("units_box")
        );
        let mut root = root.clone();
        let mut grid = root.children[1].children[1].clone();
        grid.path = "unit_counter.active_abilities".into();
        root.children.push(grid);
        assert!(
            ability_grid(&guis, &root, &mut missing, &mut Vec::new())
                .unwrap()
                .is_none()
        );
        assert!(missing.contains(&"active_ability_item.active_ability".into()));
    }

    #[test]
    fn counter_button_lookup_rejects_icon_and_checkbox_arrays() {
        content_reader_script::test_game::enter();
        for kind in ["iconType", "checkboxType"] {
            let guis = native::read(format!(
                "guiTypes={{containerWindowType={{name=test {kind}={{name=count_bg spriteType=GFX_wrong}}}}}}"
            ).as_bytes());
            let mut missing = Vec::new();
            let plate = child(
                guis.template("test").unwrap(),
                native::Kind::Button,
                "count_bg",
                "test",
                &mut missing,
            )
            .unwrap();
            assert!(plate.sprite.is_empty(), "{kind}");
            assert_eq!(missing, ["test.count_bg"], "{kind}");
        }
    }

    #[test]
    fn unported_counter_button_draw_forms_are_refused_by_name() {
        content_reader_script::test_game::enter();
        for (field, name) in [
            ("centerPosition=yes", "centerPosition"),
            ("rotation=1", "rotation"),
            ("enabled=no", "enabled=no"),
            ("font=Main_14", "font"),
            ("quadTextureSprite=\"\"", "empty sprite"),
        ] {
            let guis = native::read(format!(
                "guiTypes={{containerWindowType={{name=test buttonType={{name=count_bg quadTextureSprite=GFX_plate {field}}}}}}}"
            ).as_bytes());
            let why = child(
                guis.template("test").unwrap(),
                native::Kind::Button,
                "count_bg",
                "test",
                &mut Vec::new(),
            )
            .unwrap_err()
            .to_string();
            assert!(why.contains("test.count_bg") && why.contains(name), "{why}");
            assert!(why.contains("button"), "{why}");
        }
    }
}
