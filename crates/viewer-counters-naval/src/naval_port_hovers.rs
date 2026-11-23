
use content_reader_core::gui::native;
use std::collections::BTreeMap;
use std::sync::Arc;
use viewer_native_gui::counter_hovers::gui_authored::Authored;
use viewer_native_gui::counter_hovers::task_force_port::{Control, Hover, Metadata};
use viewer_native_gui::native_gui;

fn control(window: &native::Element, element: &native::Element) -> Option<Control> {
    use native::Kind::{Button, Container, Icon};
    [
        (Button, "count_bg", Control::Normal),
        (Icon, "count_selected", Control::Normal),
        (Icon, "bar_str", Control::Normal),
        (Icon, "bar_org", Control::Normal),
        (Icon, "type", Control::Normal),
        (Icon, "flag", Control::Normal),
        (Icon, "no_orders", Control::NoOrders),
        (Icon, "invalid_orders", Control::InvalidOrders),
        (Icon, "mission_type", Control::Normal),
        (Button, "air_base_icon", Control::Carrier),
        (Icon, "pride_of_the_fleet_icon", Control::Pride),
    ]
    .into_iter()
    .find_map(|(kind, name, control)| {
        window
            .find(kind, name)
            .filter(|bound| std::ptr::eq(*bound, element))
            .map(|_| control)
    })
    .or_else(|| {
        let spotting = window.find(Container, "spotting_bar_container")?;
        let foreground = spotting
            .find(Container, "spotting_bar_fg_container")
            .and_then(|container| container.find(Icon, "spotting_bar_fg"));
        let background = spotting.find(Icon, "spotting_bar_bg");
        if std::ptr::eq(spotting, element)
            || foreground.is_some_and(|bound| std::ptr::eq(bound, element))
            || background.is_some_and(|bound| std::ptr::eq(bound, element))
        {
            return Some(Control::Spotting);
        }
        let mut pending: Vec<_> = spotting.children.iter().collect();
        while let Some(child) = pending.pop() {
            if std::ptr::eq(child, element) {
                return Some(Control::Normal);
            }
            pending.extend(&child.children);
        }
        None
    })
}

#[allow(clippy::too_many_arguments)]
pub fn presented(
    window: &native::Element,
    layout: &native_gui::Layout,
    holder: &native_gui::Holder,
    place: [i32; 2],
    origin: [i32; 2],
    resources: &dyn native_gui::Resources,
    unit_hit: usize,
    members: &Arc<[u64]>,
    metadata: &Metadata,
    notes: &mut BTreeMap<String, usize>,
) -> Vec<Hover> {
    let _observe_profile = viewer_runtime::profiling::span("dc_observe");
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
    placed
        .regions
        .iter()
        .filter_map(|region| {
            let element = viewer_counter_kit::gui_windows::find_path(window, &region.path)?;
            let bound = control(window, element);
            let hit =
                viewer_counter_kit::gui_windows::native_hit_rectangle(element, &placed, origin, resources, notes)?;
            Some(Hover {
                control: bound.unwrap_or(Control::Unbound),
                unit_hit,
                rect: hit.rect,
                supported: hit.supported,
                hit: hit.geometry,
                authored: Authored::read(element, hit.disabled),
                members: Arc::clone(members),
                metadata: metadata.clone(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Arc, BTreeMap, Control, Metadata, control, native, native_gui, presented};

    #[test]
    fn port_binder_excludes_unbound_supply_marker_counts_and_wrong_authored_kind() {
        content_reader_core::test_game::enter();
        let gui = native::read(
            br#"guiTypes = { containerWindowType = {
            name = w
            buttonType = { name = count_bg }
            iconType = { name = invalid_orders }
            iconType = { name = low_supply }
            iconType = { name = group_marker }
            instantTextBoxType = { name = count_txt }
            instantTextBoxType = { name = capital_count_txt }
            iconType = { name = air_base_icon }
            buttonType = { name = flag }
        } }"#,
        );
        let window = gui.template("w").unwrap();
        let controls: Vec<_> = window
            .children
            .iter()
            .filter_map(|element| control(window, element))
            .collect();
        assert_eq!(controls, [Control::Normal, Control::InvalidOrders]);
    }

    #[test]
    fn spotting_binder_uses_nested_lookups_and_descendants_inherit_generic_handler() {
        content_reader_core::test_game::enter();
        let gui = native::read(
            br#"guiTypes = { containerWindowType = {
            name = w
            iconType = { name = spotting_bar_bg }
            containerWindowType = { name = spotting_bar_container
                iconType = { name = spotting_bar_bg }
                containerWindowType = { name = spotting_bar_fg_container
                    iconType = { name = spotting_bar_fg }
                    iconType = { name = extra }
                }
                instantTextBoxType = { name = authored }
            }
        } }"#,
        );
        let window = gui.template("w").unwrap();
        let spotting = window
            .find(native::Kind::Container, "spotting_bar_container")
            .unwrap();
        let foreground = spotting
            .find(native::Kind::Container, "spotting_bar_fg_container")
            .unwrap();
        assert_eq!(
            control(
                window,
                window.find(native::Kind::Icon, "spotting_bar_bg").unwrap()
            ),
            None
        );
        assert_eq!(control(window, spotting), Some(Control::Spotting));
        assert_eq!(
            control(
                window,
                spotting
                    .find(native::Kind::Icon, "spotting_bar_bg")
                    .unwrap()
            ),
            Some(Control::Spotting)
        );
        assert_eq!(
            control(
                window,
                foreground
                    .find(native::Kind::Icon, "spotting_bar_fg")
                    .unwrap()
            ),
            Some(Control::Spotting)
        );
        assert_eq!(control(window, foreground), Some(Control::Normal));
        assert_eq!(
            control(
                window,
                foreground.find(native::Kind::Icon, "extra").unwrap()
            ),
            Some(Control::Normal)
        );
        assert_eq!(
            control(
                window,
                spotting.find(native::Kind::TextBox, "authored").unwrap()
            ),
            Some(Control::Normal)
        );
    }

    #[test]
    fn port_child_can_hover_when_its_clipped_plate_is_empty() {
        content_reader_core::test_game::enter();
        use native_gui::lifecycle::Instance;
        use viewer_native_gui::test_support::{Fixture, VIEWPORT};
        let gui = native::read(br#"guiTypes = { containerWindowType = {
            name = w size = { width = 66 height = 33 } clipping = no
            buttonType = { name = count_bg quadTextureSprite = GFX_bg }
            buttonType = { name = air_base_icon position = { x = -16 y = 0 } quadTextureSprite = GFX_icon }
            iconType = { name = low_supply spriteType = GFX_icon alwaystransparent = yes }
        } }"#);
        let window = gui.template("w").unwrap();
        let resources = Fixture::new(&[("GFX_bg", 1, [66, 33]), ("GFX_icon", 1, [16, 10])], &[]);
        let mut instance = Instance::construct(window, VIEWPORT, &resources).unwrap();
        instance.gui_show(window, window).unwrap();
        let layout = instance
            .layout_with_regions(window, &resources, None, true)
            .unwrap()
            .unwrap();
        let holder = native_gui::Holder {
            path: "fixture.units".into(),
            child: None,
            content: [200, 200],
            origin: [30., 40.],
            scale: 2.,
            hit_scales: None,
            logical: [30, 40],
            clip: Some([10, 50, 20, 80]),
            opacity: 1.,
            before: 0,
            position: None,
        };
        let members: Arc<[u64]> = Arc::from([7]);
        let metadata = Metadata {
            country: 1,
            visibility: 250,
            enemy: Ok(false),
            hold_fire: Some(false),
            idle: Some(false),
            no_regions: Some(false),
            pride: Some(false),
        };
        let mut notes = BTreeMap::new();
        let hovers = presented(
            window,
            &layout,
            &holder,
            [5, 6],
            [100, 200],
            &resources,
            42,
            &members,
            &metadata,
            &mut notes,
        );
        let carrier = hovers
            .iter()
            .find(|hover| hover.control == Control::Carrier)
            .unwrap();
        assert_eq!(carrier.unit_hit, 42);
        assert!(carrier.supported && carrier.contains([115., 262.]));
        assert!(!carrier.contains([109., 262.]));
        let plate = viewer_native_gui::counter_hovers::unit_hits::Hit {
            plate: None,
            ours: true,
            port: true,
            rect: [140., 252., 0., 28.],
            kind: viewer_native_gui::counter_hovers::unit_hits::Kind::TaskForce,
            members,
            layer: 1,
        };
        assert!(!plate.contains([115., 262.]));
        assert!(carrier.matches(&plate));
        assert!(notes.is_empty());
    }
}
