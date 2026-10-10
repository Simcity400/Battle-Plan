//! Root-inherited naval counter hover observers from the retained GUI draw.

use super::super::counter_tooltips::task_force::{Authored, Control, Hover};
use super::*;
use crate::map::native_gui;

// These are the retained GUI draw's independent layout, placement and hit inputs.
#[allow(clippy::too_many_arguments)]
pub(super) fn presented(
    window: &native::Element,
    layout: &native_gui::Layout,
    holder: &native_gui::Holder,
    place: [i32; 2],
    origin: [i32; 2],
    resources: &dyn native_gui::Resources,
    flotilla: &naval::Flotilla,
    unit_hit: usize,
    notes: &mut BTreeMap<String, usize>,
) -> Vec<Hover> {
    let mut out = Vec::new();
    let placed = native_gui::Layout {
        size: layout.size,
        parts: holder.place(place, layout),
        regions: holder.place_regions(place, &layout.regions),
        holders: Vec::new(),
        diagnostics: Vec::new(),
    };
    // The layout retains only presented controls. Every hit forwards this window's
    // inherited handler; only the exact control bound by the game selects a special arm.
    for region in &placed.regions {
        let Some(element) = gui_windows::find_path(window, &region.path) else {
            continue;
        };
        let Some(hit) =
            gui_windows::native_hit_rectangle(element, &placed, origin, resources, notes)
        else {
            continue;
        };
        let control = [
            (native::Kind::Icon, "no_orders", Control::NoOrders),
            (native::Kind::Icon, "invalid_orders", Control::InvalidOrders),
            (native::Kind::Icon, "low_supply", Control::Supply),
            (native::Kind::Button, "air_base_icon", Control::Carrier),
            (
                native::Kind::Icon,
                "pride_of_the_fleet_icon",
                Control::Pride,
            ),
        ]
        .into_iter()
        .find_map(|(kind, name, control)| {
            window
                .find(kind, name)
                .filter(|bound| bound.path == element.path)
                .map(|_| control)
        })
        .unwrap_or(Control::Normal);
        out.push(Hover {
            control,
            unit_hit,
            rect: hit.rect,
            supported: hit.supported,
            hit: hit.geometry,
            authored: Authored::read(element, hit.disabled),
            ours: matches!(&flotilla.side, Ok(Allegiance::Ours)),
            coarse: flotilla.coarse,
            hidden: flotilla.hidden,
            members: Arc::clone(&flotilla.members),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::gui_windows::tests::{Fixture, VIEWPORT, at_sea_flotilla};
    use super::*;
    use crate::map::native_gui::lifecycle::Instance;

    #[test]
    fn naval_inherited_controls_follow_visible_attachment_order_and_the_holder_clip() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let gui = native::read_at(br#"guiTypes = {
            containerWindowType = { name = unit_counter_navy size = { width = 66 height = 33 } clipping = no
                buttonType = { name = count_bg quadTextureSprite = GFX_bg }
                buttonType = { name = air_base_icon position = { x = -16 y = 0 } quadTextureSprite = GFX_icon }
                iconType = { name = no_orders position = { x = -8 y = -3 } spriteType = GFX_icon }
                iconType = { name = invalid_orders spriteType = GFX_icon hide = yes }
                iconType = { name = group_marker spriteType = GFX_icon alwaystransparent = yes }
            } }"#, VIEWPORT);
        let window = gui.template("unit_counter_navy").unwrap();
        let fixture = Fixture::new(&[("GFX_bg", 1, [66, 33]), ("GFX_icon", 1, [16, 10])], &[]);
        let mut instance = Instance::construct(window, VIEWPORT, &fixture).unwrap();
        instance.gui_show(window, window).unwrap();
        let layout = instance
            .layout_with_regions(window, &fixture, None, true)
            .unwrap()
            .unwrap();
        let holder = native_gui::Holder {
            path: "fixture.units_box".into(),
            child: None,
            content: [200, 200],
            origin: [30., 40.],
            scale: 2.,
            hit_scales: None,
            logical: [30, 40],
            clip: Some([10, 50, 38, 80]),
            opacity: 1.,
            before: 0,
            position: None,
        };
        // Holder clips are logical-parent coordinates, translated by screen origin.
        let mut notes = BTreeMap::new();
        let flotilla = at_sea_flotilla(false);
        let hovers = presented(
            window,
            &layout,
            &holder,
            [5, 6],
            [100, 200],
            &fixture,
            &flotilla,
            3,
            &mut notes,
        );
        assert_eq!(
            hovers.iter().map(|hover| hover.control).collect::<Vec<_>>(),
            [Control::Normal, Control::Carrier, Control::NoOrders]
        );
        // A clipped edge may retain a zero-area diagnostic footprint. Its
        // native predicate remains authoritative, rather than that rectangle.
        assert_eq!(hovers[0].rect[2], 0.0);
        assert!(!hovers[0].contains([138.0, 262.0]));
        assert_eq!(hovers[1].rect, [110., 252., 28., 20.]);
        assert!(hovers[1].contains([115.0, 262.0]));
        assert!(
            hovers
                .iter()
                .all(|hover| hover.unit_hit == 3 && hover.supported)
        );
        assert!(notes.is_empty());
    }

    #[test]
    fn inherited_native_text_and_container_use_their_actual_shapes() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let gui = native::read_at(
            br#"guiTypes = {
            containerWindowType = { name = unit_counter_navy size = { width = 66 height = 33 }
                clipping = no scale = 2
                instantTextBoxType = { name = note position = { x = 20 y = 10 }
                    font = fixture_mono text = t maxWidth = 10 maxHeight = 10 }
                containerWindowType = { name = inner position = { x = 10 y = 10 }
                    size = { width = 8 height = 10 } clipping = no }
            } }"#,
            VIEWPORT,
        );
        let window = gui.template("unit_counter_navy").unwrap();
        let fixture = Fixture::new(&[], &[]);
        let mut instance = Instance::construct(window, VIEWPORT, &fixture).unwrap();
        instance.gui_show(window, window).unwrap();
        let layout = instance
            .layout_with_regions(window, &fixture, None, true)
            .unwrap()
            .unwrap();
        let holder = native_gui::Holder {
            path: "fixture.units_box".into(),
            child: None,
            content: VIEWPORT,
            origin: [0.0; 2],
            scale: 1.0,
            hit_scales: Some(Vec::new()),
            logical: [0; 2],
            clip: None,
            opacity: 1.0,
            before: 0,
            position: None,
        };
        let mut notes = BTreeMap::new();
        let hovers = presented(
            window,
            &layout,
            &holder,
            [0; 2],
            [0; 2],
            &fixture,
            &at_sea_flotilla(false),
            0,
            &mut notes,
        );
        let text = hovers
            .iter()
            .find(|hover| hover.rect == [40.0, 20.0, 20.0, 20.0])
            .unwrap();
        assert!(text.supported);
        // The native text's layout offset is unscaled, and measured font
        // height supplies its inverse pivot. This point is outside its draw box.
        assert!(text.contains([22.0, 0.0]));
        assert!(!text.contains([21.9, 0.0]));
        let container = window.find(native::Kind::Container, "inner").unwrap();
        let placed = native_gui::Layout {
            size: layout.size,
            parts: holder.place([0; 2], &layout),
            regions: holder.place_regions([0; 2], &layout.regions),
            holders: Vec::new(),
            diagnostics: Vec::new(),
        };
        let hit =
            gui_windows::native_hit_rectangle(container, &placed, [0; 2], &fixture, &mut notes)
                .unwrap();
        assert!(hit.supported && hit.geometry.is_some());
        assert!(
            hovers
                .iter()
                .any(|hover| hover.hit == hit.geometry && hover.control == Control::Normal)
        );
        assert!(notes.is_empty());
    }

    #[test]
    fn full_frame_native_hit_does_not_replace_legacy_stretched_draw_bounds() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let gui = native::read_at(
            br#"guiTypes = { containerWindowType = {
            name = unit_counter_navy size = { width = 66 height = 33 } clipping = no
            iconType = { name = bar_str spriteType = GFX_icon }
        } }"#,
            VIEWPORT,
        );
        let window = gui.template("unit_counter_navy").unwrap();
        let control = window.find(native::Kind::Icon, "bar_str").unwrap();
        let fixture = Fixture::new(&[("GFX_icon", 1, [16, 10])], &[]);
        let mut instance = Instance::construct(window, VIEWPORT, &fixture).unwrap();
        instance.gui_show(window, window).unwrap();
        instance.control(window, control).unwrap().stretch = Some(0.5);
        let layout = instance
            .layout_with_regions(window, &fixture, None, true)
            .unwrap()
            .unwrap();
        let mut notes = BTreeMap::new();
        let native =
            gui_windows::native_hit_rectangle(control, &layout, [0; 2], &fixture, &mut notes)
                .unwrap();
        let legacy =
            gui_windows::hit_rectangle(control, &layout, [0; 2], &fixture, &mut notes).unwrap();
        assert_eq!(native.rect, legacy.rect);
        assert_eq!(legacy.rect, [0.0, 2.5, 16.0, 5.0]);
        assert!(native.geometry.as_ref().unwrap().contains([4.0, 1.0]));
        assert!(!crate::combat::contains(legacy.rect, [4.0, 1.0]));
        assert!(legacy.supported && legacy.geometry.is_none());
        assert!(notes.is_empty());
    }
}
