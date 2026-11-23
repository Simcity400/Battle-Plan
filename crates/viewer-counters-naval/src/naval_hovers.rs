
use content_reader_gui::gui::native;
use std::collections::BTreeMap;
use std::sync::Arc;
use viewer_foundation::allegiance::Allegiance;
use viewer_native_gui::counter_hovers::gui_authored::Authored;
use viewer_native_gui::counter_hovers::task_force::{Control, Hover};
#[cfg(test)]
use viewer_runtime::geometry::contains;
#[cfg(test)]
use viewer_native_gui::gui_resources::hit_rectangle;
use viewer_native_gui::native_gui;

#[allow(clippy::too_many_arguments)]
pub fn presented(
    window: &native::Element,
    layout: &native_gui::Layout,
    holder: &native_gui::Holder,
    place: [i32; 2],
    origin: [i32; 2],
    resources: &dyn native_gui::Resources,
    flotilla: &viewer_counter_kit::flotilla::Flotilla,
    unit_hit: usize,
    notes: &mut BTreeMap<String, usize>,
) -> Vec<Hover> {
    let _observe_profile = viewer_runtime::profiling::span("dc_observe");
    let mut out = Vec::new();
    let placed = native_gui::Layout {
        size: layout.size,
        parts: holder.place(place, layout),
        regions: holder.place_regions(place, &layout.regions),
        holders: Vec::new(),
        diagnostics: Vec::new(),
        pointed: None,
        hovered: None,
        target: None,
        boxes: Vec::new(),
    };
    for region in &placed.regions {
        let Some(element) = viewer_counter_kit::gui_windows::find_path(window, &region.path) else {
            continue;
        };
        let Some(hit) =
            viewer_counter_kit::gui_windows::native_hit_rectangle(element, &placed, origin, resources, notes)
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
    use viewer_counter_kit::gui_windows::test_support::at_sea_flotilla;
    use super::{BTreeMap, Control, contains, hit_rectangle, native, native_gui, presented};
    use viewer_native_gui::native_gui::lifecycle::Instance;
    use viewer_native_gui::test_support::{Fixture, VIEWPORT};

    #[test]
    fn naval_inherited_controls_follow_visible_attachment_order_and_the_holder_clip() {
        content_reader_script::test_game::enter();
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
        content_reader_script::test_game::enter();
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
        assert!(text.contains([22.0, 0.0]));
        assert!(!text.contains([21.9, 0.0]));
        let container = window.find(native::Kind::Container, "inner").unwrap();
        let placed = native_gui::Layout {
            size: layout.size,
            parts: holder.place([0; 2], &layout),
            regions: holder.place_regions([0; 2], &layout.regions),
            holders: Vec::new(),
            diagnostics: Vec::new(),
            pointed: None,
            hovered: None,
            target: None,
            boxes: Vec::new(),
        };
        let hit =
            viewer_counter_kit::gui_windows::native_hit_rectangle(container, &placed, [0; 2], &fixture, &mut notes)
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
        content_reader_script::test_game::enter();
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
            viewer_counter_kit::gui_windows::native_hit_rectangle(control, &layout, [0; 2], &fixture, &mut notes)
                .unwrap();
        let legacy = hit_rectangle(control, &layout, [0; 2], &fixture, &mut notes).unwrap();
        assert_eq!(native.rect, legacy.rect);
        assert_eq!(legacy.rect, [0.0, 2.5, 16.0, 5.0]);
        assert!(native.geometry.as_ref().unwrap().contains([4.0, 1.0]));
        assert!(!contains(legacy.rect, [4.0, 1.0]));
        assert!(legacy.supported && legacy.geometry.is_none());
        assert!(notes.is_empty());
    }
}
