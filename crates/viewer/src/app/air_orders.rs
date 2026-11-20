//! Right-clicks with the viewer's air wings selected, answered as the game answers an observer:
//! the orders' sounds and the attached-wing confirmation, never an order.

use super::{Document, MapHit, Viewer};
use eframe::egui;
use std::{collections::BTreeMap, sync::Arc};
use viewer_combat::confirmation::{Confirmation, ConfirmationDrawn, Control};
use viewer_counters::map::{
    air_orders::{CONFIRMATION_DESCRIPTION, CONFIRMATION_TITLE, Response},
    counters::CounterReport,
};
use viewer_foundation::map::native_gui::pointer::Input;

/// The map icon a right button change came over, named as the frame on screen showed it, so a
/// draw between the press, the release and its answer renames nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Under {
    /// An air base button: its province and the kind of base it shows.
    AirBase { province: u32, kind: i32 },
    /// Any other icon.
    Icon,
}

impl Under {
    /// `hit`, resolved against `report`, the counters' report it was resolved in.
    pub(super) fn of(hit: MapHit, report: &CounterReport) -> Self {
        match hit {
            MapHit::Base(index) => report.air_base_hits.get(&index).map_or(Self::Icon, |&(province, kind)| Self::AirBase { province, kind }),
            _ => Self::Icon,
        }
    }
}

#[derive(Default)]
pub(super) struct AirOrders {
    /// What a held right press went down on, on the map and off every window.
    pressed: Option<Option<Under>>,
    confirmation: Confirmation,
    open: bool,
    /// The open pop-up's controls as last drawn.
    controls: Vec<([f32; 4], Control)>,
    /// What the last order could not decide, and what the pop-up refused, by name.
    refused: BTreeMap<String, usize>,
    drawn_refused: BTreeMap<String, usize>,
}

impl AirOrders {
    /// Whether the open pop-up covers `at`.
    pub(super) fn over(&self, at: [f32; 2]) -> bool {
        self.open && self.controls.iter().any(|(rect, _)| viewer_foundation::map::gui_resources::contains(*rect, at))
    }

    pub(super) fn holding(&self) -> bool {
        self.open && self.confirmation.holding()
    }

    /// A right button change at a point on the map off every window, `under` there: what a
    /// release orders on when its press went down on the map too.
    pub(super) fn secondary(&mut self, pressed: bool, free: bool, under: Option<Under>) -> Option<Option<Under>> {
        if pressed {
            self.pressed = free.then_some(under);
            return None;
        }
        let from = self.pressed.take()?;
        match (from, under) {
            _ if !free => None,
            (Some(Under::AirBase { .. }), Some(Under::AirBase { .. })) if from == under => Some(under),
            (_, None) => Some(None),
            _ => None,
        }
    }

    /// The button let go without a release on the map.
    pub(super) fn release(&mut self) {
        self.pressed = None;
    }

    /// An order's answer: what it could not decide, and whether the pop-up opens.
    pub(super) fn answered(&mut self, response: Response) {
        self.refused = response.refused.into_iter().map(|reason| (reason, 1)).collect();
        self.open |= response.confirmation;
    }

    pub(super) fn warnings(&self) -> impl Iterator<Item = String> + '_ {
        self.refused
            .iter()
            .chain(&self.drawn_refused)
            .map(|(reason, count)| format!("Air orders: {reason} ({count} refused)"))
    }
}

impl Viewer {
    /// The answer to a right release at `point`, whose press went down on `under`, over the rows
    /// of the hour it was made at.
    #[allow(clippy::too_many_arguments, reason = "a click's whole context, as its siblings take it")]
    pub(super) fn air_order(&mut self, document: &Document, point: [f32; 2], under: Option<Under>, order: usize, shift: bool, pick: impl FnOnce([f32; 2]) -> Option<u32>, ctx: &egui::Context) {
        let wings = Arc::clone(self.air_selection.wings());
        let region = match under {
            Some(Under::AirBase { .. }) => None,
            None if !wings.is_empty() => {
                let Some(region) = pick(point).and_then(|province| document.content.counters.region(province)) else {
                    return;
                };
                Some(region)
            }
            _ => return,
        };
        self.click(document, super::held_clicks::Click::AirOrder { region, wings, shift, order }, None, ctx);
    }

    /// The open pop-up's keys: an authored shortcut presses its button.
    pub(super) fn confirmation_keys(&mut self, ui: &egui::Ui, document: &Document, extent: [f32; 2]) {
        if !self.air_orders.open {
            return;
        }
        let Ok(shortcuts) = Confirmation::shortcuts(&document.combat, extent) else {
            return;
        };
        let pressed = ui.input(|input| {
            input.events.iter().any(|event| match event {
                egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } => shortcuts.iter().any(|(_, text)| {
                    super::picker::parse_shortcut(text).is_some_and(|(wanted, wanted_key)| wanted_key == *key && modifiers.matches_exact(wanted))
                }),
                _ => false,
            })
        });
        if pressed {
            self.air_orders.open = false;
        }
    }

    /// The open pop-up, drawn read-only; none while closed.
    pub(super) fn draw_confirmation(&mut self, document: &Document, extent: [f32; 2], time: f32, input: Option<Input>) -> Option<ConfirmationDrawn> {
        let orders = &mut self.air_orders;
        if !orders.open {
            orders.controls.clear();
            orders.drawn_refused.clear();
            return None;
        }
        let localise = |key: &str| document.combat.localisation.localise_with(key.as_bytes(), &[]).map_err(|error| format!("{key}: {error}"));
        let drawn = match (localise(CONFIRMATION_TITLE), localise(CONFIRMATION_DESCRIPTION)) {
            (Ok(title), Ok(description)) => orders.confirmation.draw(&document.combat, &title, &description, extent, true, time, true, input).map_err(|error| error.to_string()),
            (Err(why), _) | (_, Err(why)) => Err(why),
        };
        match drawn {
            Ok(drawn) => {
                orders.controls.clone_from(&drawn.controls);
                orders.drawn_refused.clone_from(&drawn.drawn.refused);
                Some(drawn)
            }
            Err(why) => {
                orders.controls.clear();
                orders.drawn_refused = BTreeMap::from([(format!("the attached-wing confirmation is not drawn: {why}"), 1)]);
                None
            }
        }
    }

    /// A pop-up button clicked: the order it confirms is never posted, so it closes.
    pub(super) fn confirmation_click(&mut self, control: Control) {
        if control != Control::Window {
            self.air_orders.open = false;
        }
    }

    pub(super) fn confirmation_pointing(&self) -> bool {
        self.air_orders.open && self.air_orders.confirmation.pointing()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_right_release_orders_only_on_what_its_press_went_down_on() {
        let base = |province| Some(Under::AirBase { province, kind: 0 });
        let mut orders = AirOrders::default();
        assert_eq!(orders.secondary(false, true, None), None);
        orders.secondary(true, true, base(3));
        assert_eq!(orders.secondary(false, true, base(3)), Some(base(3)));
        orders.secondary(true, true, base(3));
        assert_eq!(orders.secondary(false, true, base(4)), None);
        orders.secondary(true, true, Some(Under::Icon));
        assert_eq!(orders.secondary(false, true, Some(Under::Icon)), None, "only an air base takes the order");
        orders.secondary(true, true, None);
        assert_eq!(orders.secondary(false, true, None), Some(None));
        // A press over a window, or a release over one, orders nothing.
        orders.secondary(true, false, None);
        assert_eq!(orders.secondary(false, true, None), None);
        orders.secondary(true, true, None);
        assert_eq!(orders.secondary(false, false, None), None);
    }

    /// A new hour drawn between the press and the release numbers the bases afresh: the base is
    /// named by what the frame on screen showed at each, never by its place in a draw's list.
    #[test]
    fn a_draw_between_the_press_and_the_release_renames_no_air_base() {
        let report = |bases: &[(usize, u32)]| CounterReport { air_base_hits: bases.iter().map(|&(index, province)| (index, (province, 1))).collect(), ..CounterReport::default() };
        let (pressed, released) = (report(&[(3, 40), (5, 41)]), report(&[(3, 41), (5, 40)]));
        let mut orders = AirOrders::default();
        orders.secondary(true, true, Some(Under::of(MapHit::Base(3), &pressed)));
        assert_eq!(orders.secondary(false, true, Some(Under::of(MapHit::Base(3), &released))), None, "another base now stands at that place in the list");
        orders.secondary(true, true, Some(Under::of(MapHit::Base(3), &pressed)));
        assert_eq!(orders.secondary(false, true, Some(Under::of(MapHit::Base(5), &released))), Some(Some(Under::AirBase { province: 40, kind: 1 })));
        assert_eq!(Under::of(MapHit::Base(7), &released), Under::Icon, "a naval base or an air grid");
    }
}
