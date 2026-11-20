//! The minimap and the find view over a map view, drawn with the picker and sharing its pointer:
//! the minimap's slide, its world picture's press, drag and wheel, its ping mode, and the find
//! view's focus and results.
//!
//! A ping placed in ping mode, on the minimap or on the map, is a game command: the replay
//! disarms the mode as the game does and posts nothing.

use super::Viewer;
use crate::document::Document;
use eframe::egui;
use installed_content_localisation::localisation::Localisation;
use std::collections::BTreeMap;
use viewer_combat::{
    find_view,
    minimap::{self, Armed, Control, Slide},
    picker::{Control as PickerControl, PickerDrawn, Placed},
};
use viewer_foundation::map::{
    camera::{Camera, motion::Floor},
    native_gui::pointer::Input,
};
use viewer_playback::cursors::MapCursor;

/// The runtime DLC bit of the feature the map's ping mode and its cursors need.
const PING_DLC: u32 = 0x8;

#[derive(Default)]
pub struct MinimapState {
    pub view: minimap::Minimap,
    pub slide: Slide,
    /// The armed ping button.
    armed: Armed,
    /// A press on the world picture is held: the pointer moves the camera's target.
    dragging: bool,
    /// A press on the map placed a ping: its release is no click.
    swallow: bool,
    /// Ping mode as this frame's keys and armed button give it.
    ping: Option<bool>,
    /// The world picture as last drawn: `x, y, w, h` in interface pixels.
    image: Option<[i32; 4]>,
    /// The animation clock the last draw read, which a toggle starts its slide at.
    now: f32,
    /// The recording's runtime DLC mask at the head, kept while its rows load.
    dlc: Option<u32>,
    /// Why parts of the minimap are not shown.
    pub refused: BTreeMap<String, usize>,
}

impl MinimapState {
    pub fn holding(&self) -> bool {
        self.dragging || self.view.holding()
    }

    pub fn active(&self) -> bool {
        self.slide.sliding() || self.view.pointing() || self.view.active()
    }

    /// Arms the offensive or the defensive ping, as its button does.
    pub fn arm(&mut self, offensive: bool) {
        self.armed = Some(offensive);
    }
}

/// A localised string, or `None` with the reason in `refused` when the localisation cannot give it.
fn text(localisation: &Localisation, key: &str, refused: &mut BTreeMap<String, usize>) -> Option<String> {
    localisation
        .localise(key.as_bytes())
        .map_err(|why| refused.insert(why.to_string(), 1))
        .ok()
}

fn topmost(controls: &[Placed], at: [f32; 2]) -> Option<&PickerControl> {
    controls
        .iter()
        .rev()
        .find(|placed| placed.contains(at))
        .map(|placed| &placed.control)
}

impl Viewer {
    /// Flips the minimap open or closed, sliding by its defines.
    pub(super) fn toggle_minimap(&mut self, document: &Document) {
        if let Ok(defines) = &document.minimap_defines {
            let state = &mut self.picker.minimap;
            state.slide.toggle(state.now, defines.duration);
        }
    }

    /// The ping cursor the map shows: in ping mode, with the feature that owns pings.
    pub(crate) fn ping_cursor(&self) -> Option<MapCursor> {
        let state = &self.picker.minimap;
        let owned = state.dlc.is_some_and(|mask| mask & PING_DLC != 0);
        let offensive = state.ping.filter(|_| owned)?;
        Some(if offensive {
            MapCursor::PingOffensive
        } else {
            MapCursor::PingDefensive
        })
    }

    /// The minimap's and the find view's share of the frame's pointer, before the camera moves.
    /// A press on the world picture outside ping mode moves the camera's target there and holds
    /// it to the pointer until released; in ping mode it places a ping. The wheel over the
    /// picture zooms about the target, then moves it to the pointer; over the results it scrolls
    /// them. A press on the bare map in ping mode places a ping and is no click: `true` while
    /// the frame's map clicks are to be dropped. Then the find view's asked move.
    #[allow(
        clippy::too_many_arguments,
        reason = "one frame's view, pointer and clock, as the picker's draw takes them"
    )]
    pub(super) fn map_windows_pointer(
        &mut self,
        ui: &egui::Ui,
        document: &Document,
        camera: &mut Camera,
        floor: Floor<'_>,
        controls: &[Placed],
        point: Option<[f32; 2]>,
        presses: &[([f32; 2], bool, egui::Modifiers)],
        held: bool,
        extent: [f32; 2],
        gui_at: &dyn Fn([f32; 2]) -> bool,
    ) -> bool {
        let map = [document.map.width as i32, document.map.height as i32];
        let head = super::head_rows::head(super::live_of(&self.playhead_map));
        if let Some(Some(mask)) = self.head_rows.map(head, None, |frame| {
            frame
                .complete::<replay_data::rows::tables::SimulationFeatures>()
                .ok()
                .and_then(|rows| rows.first().map(|row| row.dlc_mask))
        }) {
            self.picker.minimap.dlc = Some(mask);
        }
        let modifiers = ui.input(|input| input.modifiers);
        let state = &mut self.picker.minimap;
        let ping_with = |armed: Armed, modifiers: egui::Modifiers| minimap::ping_mode(armed, modifiers.ctrl, modifiers.alt, modifiers.shift);
        state.ping = ping_with(state.armed, modifiers);
        let owned = state.dlc.is_some_and(|mask| mask & PING_DLC != 0);
        let mut dropped = false;
        for &(at, pressed, held_with) in presses {
            let ping = |armed: Armed| ping_with(armed, held_with);
            if !pressed {
                state.dragging = false;
                dropped |= std::mem::take(&mut state.swallow);
                continue;
            }
            match topmost(controls, at) {
                Some(PickerControl::Minimap(Control::World)) => {
                    if ping(state.armed).is_some() {
                        state.armed = None;
                    } else {
                        state.dragging = true;
                    }
                }
                None if owned && !gui_at(at) && ping(state.armed).is_some() => {
                    state.armed = None;
                    state.swallow = true;
                }
                _ => {}
            }
        }
        if !held {
            state.dragging = false;
        }
        let image = state.image;
        if state.dragging
            && let (Some(point), Some(image)) = (point, image)
        {
            self.camera_manual = true;
            let at = minimap::world_point(point, image, map);
            self.zoom.place(camera, at, None, floor);
        }
        let over = point.and_then(|at| topmost(controls, at).cloned());
        let lines: Vec<f32> = ui.input(|input| {
            input
                .raw
                .events
                .iter()
                .filter_map(|event| match event {
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Line,
                        delta,
                        ..
                    } => Some(delta.y),
                    _ => None,
                })
                .collect()
        });
        match (over, point) {
            (Some(PickerControl::Minimap(Control::World)), Some(point)) if !lines.is_empty() => {
                let notches = self.zoom.wheel_events(lines);
                if !notches.is_empty()
                    && let Some(image) = image
                {
                    self.camera_manual = true;
                    for notch in notches {
                        self.zoom.zoom_step(camera, notch > 0, floor);
                    }
                    let at = minimap::world_point(point, image, map);
                    self.zoom.place(camera, at, None, floor);
                }
            }
            (Some(PickerControl::Find(_)), Some(point)) if !lines.is_empty() => {
                let notches: i32 = self.zoom.wheel_events(lines).iter().sum();
                if notches != 0
                    && let Err(error) =
                        self.picker
                            .find
                            .view
                            .wheel(&document.combat, extent, point, notches)
                {
                    self.picker.find.refused.insert(error.to_string(), 1);
                }
            }
            _ => {}
        }
        self.find_view_move(document, camera, floor);
        dropped
    }

    /// Draws the minimap and, while open, the find view over the picker's frame, their controls
    /// after the picker's.
    #[allow(
        clippy::too_many_arguments,
        reason = "one frame's view, pointer and clock, as the picker's draw takes them"
    )]
    pub(super) fn draw_map_windows(
        &mut self,
        document: &Document,
        extent: [f32; 2],
        hovers: bool,
        time: f32,
        advance: bool,
        input: Option<Input>,
        out: &mut PickerDrawn,
    ) {
        self.picker.minimap.now = time;
        let picker_rects: Vec<[f32; 4]> = out.controls.iter().map(|placed| placed.rect).collect();
        let free = |at: [f32; 2]| {
            !picker_rects
                .iter()
                .any(|&rect| viewer_foundation::map::gui_resources::contains(rect, at))
        };
        let input = input.map(|input| input.filter(free));
        let footprint = self.camera.map(|placement| {
            let camera = placement.on(
                viewer_foundation::map::camera::MapSize::of(&document.map),
                document.map_defines,
            );
            viewer_foundation::map::province_admission::water_footprint(
                camera,
                extent.map(|value| value.max(1.0) as u32),
            )
        });
        let mut refused = BTreeMap::new();
        let slide = match &document.minimap_defines {
            Ok(defines) => self.picker.minimap.slide.offset(time, *defines),
            Err(why) => {
                refused.insert(format!("its slide is withheld: {why}"), 1);
                0
            }
        };
        let localisation = &document.combat.localisation;
        let handle = text(localisation, "SEND_MINIMAP_HANDLE_TITLE", &mut refused);
        let offensive = [
            text(localisation, "SEND_OFFENSIVE_PING_MINIMAP_TITLE", &mut refused),
            text(localisation, "SEND_OFFENSIVE_PING_MINIMAP_DESCRIPTION", &mut refused),
        ];
        let defensive = [
            text(localisation, "SEND_DEFENSIVE_PING_MINIMAP_TITLE", &mut refused),
            text(localisation, "SEND_DEFENSIVE_PING_MINIMAP_DESCRIPTION", &mut refused),
        ];
        let state = minimap::State {
            footprint,
            map: [document.map.width as i32, document.map.height as i32],
            slide,
            handle_tooltip: handle.as_deref(),
            offensive_tooltip: offensive.each_ref().map(Option::as_deref),
            defensive_tooltip: defensive.each_ref().map(Option::as_deref),
            ping: self.picker.minimap.ping,
        };
        match self.picker.minimap.view.draw(
            &document.combat,
            &state,
            extent,
            hovers,
            time,
            advance,
            input.clone(),
            &mut out.drawn,
            &mut refused,
        ) {
            Ok(drawn) => {
                self.picker.minimap.image = drawn.image;
                out.controls.extend(drawn.controls);
                out.clicked.extend(drawn.clicked);
            }
            Err(error) => {
                self.picker.minimap.image = None;
                refused.insert(error.to_string(), 1);
            }
        }
        self.picker.minimap.refused = refused;
        if !self.picker.find.open {
            return;
        }
        let find = &self.picker.find;
        let mut refused = BTreeMap::new();
        let shown = if find.field.text().is_empty() && !find.field.focused {
            text(&document.combat.localisation, "TYPE_HERE", &mut refused).unwrap_or_default()
        } else {
            find.field.text().to_owned()
        };
        let query = find.field.text().to_owned();
        let results = find.results();
        let state = find_view::State {
            text: &shown,
            query: &query,
            caret: find.field.caret(time),
            results: &results,
            fingerprint: find.fingerprint(),
        };
        match self.picker.find.view.draw(
            &document.combat,
            &state,
            extent,
            hovers,
            time,
            advance,
            input,
            &mut out.drawn,
            &mut refused,
        ) {
            Ok(drawn) => {
                if let Some(caret) = drawn.caret {
                    self.picker.find.field.press(caret, time);
                }
                out.controls.extend(drawn.controls);
                out.clicked.extend(drawn.clicked);
            }
            Err(error) => {
                refused.insert(error.to_string(), 1);
            }
        }
        self.picker.find.window_refused = refused;
    }

    /// The minimap's and the find view's refusal lines, each under its own name.
    pub(super) fn map_window_warnings(&self) -> Vec<String> {
        let lines = |surface: &str, refused: &BTreeMap<String, usize>| {
            refused
                .iter()
                .map(|(reason, count)| format!("{surface}: {reason} ({count} refused)"))
                .collect::<Vec<_>>()
        };
        let mut out = lines("Minimap", &self.picker.minimap.refused);
        if self.picker.find.open {
            out.extend(lines("Find view", &self.picker.find.refused));
            out.extend(lines("Find view", &self.picker.find.window_refused));
        }
        out
    }

    /// The animation clock the minimap and the find view last drew at.
    pub(super) fn picker_now(&self) -> f32 {
        self.picker.minimap.now
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_string_the_localisation_cannot_give_is_refused_by_name_and_shows_nothing() {
        let localisation = Localisation::default();
        let mut refused = BTreeMap::new();
        assert_eq!(text(&localisation, "TYPE_HERE", &mut refused).as_deref(), Some("TYPE_HERE"), "a key the content lacks is drawn as its own text, as the game does");
        assert!(refused.is_empty());
        let cut = format!("{}§", "X".repeat(0x17FF));
        assert_eq!(text(&localisation, &cut, &mut refused), None);
        assert!(refused.keys().all(|reason| reason.starts_with("localisation XXX")) && refused.len() == 1, "{refused:?}");
    }
}
