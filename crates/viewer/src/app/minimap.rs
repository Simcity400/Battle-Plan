
use super::Viewer;
use crate::document::Document;
use eframe::egui;
use content_reader_localisation::localisation::Localisation;
use std::collections::BTreeMap;
use viewer_combat::{
    find_view,
    minimap::{self, Armed, Control, Slide},
    picker::{Control as PickerControl, PickerDrawn, Placed},
};
use viewer_gpu::camera::{Camera, motion::Floor};
use viewer_native_gui::native_gui::pointer::Input;
use viewer_playback::cursors::MapCursor;

const PING_DLC: u32 = 0x8;

#[derive(Default)]
pub struct MinimapState {
    pub view: minimap::Minimap,
    pub slide: Slide,
    armed: Armed,
    dragging: bool,
    swallow: bool,
    ping: Option<bool>,
    image: Option<[i32; 4]>,
    now: f32,
    dlc: Option<u32>,
    pub refused: BTreeMap<String, usize>,
}

impl MinimapState {
    pub fn holding(&self) -> bool {
        self.dragging || self.view.holding()
    }

    pub fn active(&self) -> bool {
        self.slide.sliding() || self.view.pointing() || self.view.active()
    }

    pub fn arm(&mut self, offensive: bool) {
        self.armed = Some(offensive);
    }
}

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
    pub(super) fn toggle_minimap(&mut self, document: &Document) {
        if let Ok(defines) = &document.minimap_defines {
            let state = &mut self.picker.minimap;
            state.slide.toggle(state.now, defines.duration);
        }
    }

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

    #[expect(
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

    #[expect(
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
                .any(|&rect| viewer_runtime::geometry::contains(rect, at))
        };
        let input = input.map(|input| input.filter(free));
        let footprint = self.camera.map(|placement| {
            let camera = placement.on(
                viewer_gpu::camera::MapSize::of(&document.map),
                document.map_defines,
            );
            viewer_gpu::province_admission::water_footprint(
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
