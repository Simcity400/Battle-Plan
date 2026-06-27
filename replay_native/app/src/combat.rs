//! STAGE 4 app-shell integration of the COMBAT-PANEL adapter.
//!
//! [`CombatShell`] owns the static province CENTROIDS + NAMES (loaded once), and —
//! per scrub tick — the aggregated combat view-models for the current hour. It
//! lays out clickable BUBBLES at each combat's `location_province` on the LIVE map
//! (using the app's [`MapCamera`] so bubbles track pan/zoom exactly), and — when a
//! bubble is clicked — opens the stacked, read-only COMBAT PANEL for that combat.
//!
//! All input is READ-ONLY: bubble pick opens the panel; the panel's close control
//! closes it. Nothing mutates the snapshot or the game. The view-models are rebuilt
//! (never mutated in place) on every scrub tick — the no-fabrication immutability
//! rule — and every battle-DETAIL field stays badged UNAVAILABLE.

use std::collections::HashMap;

use combat_panel::bubbles::{pick_bubble, Bubble, ProvinceCentroids};
use combat_panel::drill::{CombatDrill, CombatHit, PanelAction};
use combat_panel::{aggregate_combats, CombatDomain, CombatVM, ProvinceNames, SideTag};
use snapshot::Snapshot;

use crate::camera::MapCamera;

/// The close-control leaf names the panel's "X" routes through.
const CLOSE_NAMES: &[&str] = &["close", "close_button", "X"];

pub struct CombatShell {
    centroids: Option<ProvinceCentroids>,
    names: ProvinceNames,
    drill: CombatDrill,

    /// the aggregated land combats for the current hour (rebuilt per tick).
    combats: Vec<CombatVM>,
    /// the open combat's id (the stacked panel), or None.
    open: Option<String>,
    date: String,
    /// holding-tag -> resolved ruling-party ideology (for the flag) at this hour.
    ideology: HashMap<String, String>,
}

impl CombatShell {
    /// Build with the static map artifacts. `centroids` may be `None` if the raster
    /// is unavailable (bubbles then can't be placed — the panel still works once a
    /// combat is selected by other means, but normally the raster is present).
    pub fn new(centroids: Option<ProvinceCentroids>, names: ProvinceNames) -> CombatShell {
        CombatShell {
            centroids,
            names,
            drill: CombatDrill::new(CLOSE_NAMES.iter().map(|s| s.to_string()).collect()),
            combats: Vec::new(),
            open: None,
            date: String::new(),
            ideology: HashMap::new(),
        }
    }

    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }
    pub fn date(&self) -> &str {
        &self.date
    }
    pub fn combat_count(&self) -> usize {
        self.combats.len()
    }

    /// Rebuild the combat view-models from the snapshot at the current hour. Called
    /// on startup + every scrub tick. If the previously-open combat no longer
    /// exists (battle ended), the panel is auto-closed (no stale-target render).
    pub fn rebind(&mut self, snap: &Snapshot) {
        self.date = snap.date.clone().unwrap_or_default();

        // Assemble the domains the combat reader needs into a Value (the snapshot's
        // payload domains are raw Values — no full re-serialize needed).
        let mut obj = serde_json::Map::new();
        for key in ["combats", "map", "provinces"] {
            if let Some(v) = snap.domain(key) {
                obj.insert(key.to_string(), v.clone());
            }
        }
        let snap_value = serde_json::Value::Object(obj);
        let domain = CombatDomain::from_snapshot_value(&snap_value);

        let controller_of = |p: i64| domain.controller_of(p);
        let province_name = |p: i64| self.names.get(p);
        let combats_json = snap_value
            .get("combats")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        self.combats =
            aggregate_combats(&combats_json, Some("land"), &controller_of, &province_name);

        // resolve each holding tag's ruling ideology for the flag (derived).
        self.ideology.clear();
        for vm in &self.combats {
            if let SideTag::HoldingTagDerived(t) = &vm.defender.tag {
                if !self.ideology.contains_key(t) {
                    if let Some(id) = snap
                        .country(t)
                        .and_then(|c| c.get("politics"))
                        .and_then(|p| p.get("ruling_party"))
                        .and_then(|r| r.as_str())
                    {
                        self.ideology.insert(t.clone(), id.to_string());
                    }
                }
            }
        }

        // auto-close a panel whose combat ended (lifecycle: combat_id vanished).
        if let Some(id) = &self.open {
            if !self.combats.iter().any(|c| &c.combat_id == id) {
                self.open = None;
            }
        }
    }

    /// Lay out the bubbles in SCREEN space from the live map camera. Empty if the
    /// raster (centroids) is unavailable. A combat whose province is absent from
    /// the raster is skipped (no fabricated position).
    pub fn bubbles(&self, cam: &MapCamera) -> Vec<Bubble> {
        let cent = match &self.centroids {
            Some(c) => c,
            None => return Vec::new(),
        };
        let mut out = Vec::new();
        for vm in &self.combats {
            if vm.location_province < 0 {
                continue;
            }
            let c = match cent.get(vm.location_province as u32) {
                Some(c) => c,
                None => continue,
            };
            let (sx, sy) = cam.raster_to_screen(c.0, c.1);
            let total = vm.attacker.ref_count() + vm.defender.ref_count();
            let radius = (8.0 + (total as f32).sqrt() * 3.0).clamp(8.0, 28.0);
            out.push(Bubble {
                combat_id: vm.combat_id.clone(),
                sx,
                sy,
                radius,
                defender_tag: vm.defender.tag.tag().map(|t| t.to_string()),
            });
        }
        out
    }

    /// Try to OPEN a panel from a map click at screen `(x,y)`: if it lands on a
    /// bubble, route bubble→open through the drill registry. Returns true if a
    /// bubble was hit (so the app consumes the click instead of map-picking).
    pub fn try_open_at(&mut self, cam: &MapCamera, x: f32, y: f32) -> bool {
        let bubbles = self.bubbles(cam);
        if let Some(id) = pick_bubble(&bubbles, x, y) {
            match self.drill.dispatch(&CombatHit::Bubble { combat_id: id }) {
                PanelAction::OpenCombatPanel { combat_id } => {
                    self.open = Some(combat_id);
                    return true;
                }
                _ => {}
            }
        }
        false
    }

    /// Route a click while a panel is open: returns true (consumed) iff it hit the
    /// close control, closing the panel. The app passes the close button's rect.
    pub fn try_close_at(&mut self, close_rect: (f32, f32, f32, f32), x: f32, y: f32) -> bool {
        if self.open.is_none() {
            return false;
        }
        let (cx, cy, cw, ch) = close_rect;
        if x >= cx && x < cx + cw && y >= cy && y < cy + ch {
            match self.drill.dispatch(&CombatHit::Control {
                name: "close".into(),
            }) {
                PanelAction::CloseCombatPanel => {
                    self.open = None;
                    return true;
                }
                _ => {}
            }
        }
        false
    }

    /// Explicit close (e.g. Esc / toggle key).
    pub fn close(&mut self) {
        self.open = None;
    }

    /// Open the FIRST aggregated combat's panel (used by the headless `--shot-combat`
    /// proof so the stacked panel is in the screenshot). No-op if there are none.
    pub fn open_first(&mut self) {
        if let Some(first) = self.combats.first() {
            self.open = Some(first.combat_id.clone());
        }
    }

    /// The currently-open combat view-model + its holding-tag ideology (for the
    /// flag), if a panel is open and the combat still exists.
    pub fn open_combat(&self) -> Option<(&CombatVM, Option<&str>)> {
        let id = self.open.as_ref()?;
        let vm = self.combats.iter().find(|c| &c.combat_id == id)?;
        let ideo = match &vm.defender.tag {
            SideTag::HoldingTagDerived(t) => self.ideology.get(t).map(|s| s.as_str()),
            SideTag::Unavailable => None,
        };
        Some((vm, ideo))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn snap(combats: serde_json::Value) -> Snapshot {
        let v = json!({
            "date": "1941.4.13.11",
            "combats": combats,
            "provinces": { "12740": { "controller": "ENG" } },
            "map": { "control_by_province": { "12740": "ENG" } },
            "countries": { "ENG": { "politics": { "ruling_party": "democratic" } } }
        });
        serde_json::from_value(v).unwrap()
    }

    fn one_combat() -> serde_json::Value {
        json!([{
            "combat_id":"5457:62","kind":"land","location_province":12740,
            "attackers":[[7989,4713]],
            "defenders":[[2528,4713],[4030,4713]],
            "combat_width":null,"terrain":"hills","progress":14.5,
            "modifiers":{"day":44,"duration":44}
        }])
    }

    #[test]
    fn rebind_aggregates_and_resolves_ideology() {
        let mut shell = CombatShell::new(None, ProvinceNames::default());
        shell.rebind(&snap(one_combat()));
        assert_eq!(shell.combat_count(), 1);
        assert_eq!(shell.date(), "1941.4.13.11");
        assert_eq!(
            shell.ideology.get("ENG").map(|s| s.as_str()),
            Some("democratic")
        );
    }

    #[test]
    fn bubble_pick_opens_panel() {
        // a 2x1 raster: province 12740 fills it -> centroid (0.5, 0.0)
        let raster = render::map::ProvinceRaster {
            width: 2,
            height: 1,
            ids: vec![12740, 12740],
            max_id: 12740,
        };
        let cent = ProvinceCentroids::from_raster(&raster);
        let mut shell = CombatShell::new(Some(cent), ProvinceNames::default());
        shell.rebind(&snap(one_combat()));
        // full-world camera over a 2x1 raster on a 200x100 screen.
        let cam = MapCamera::new(200, 100, 2, 1);
        let bubbles = shell.bubbles(&cam);
        assert_eq!(bubbles.len(), 1);
        let (bx, by) = (bubbles[0].sx, bubbles[0].sy);
        // click ON the bubble -> opens
        assert!(shell.try_open_at(&cam, bx, by));
        assert!(shell.is_open());
        let (vm, ideo) = shell.open_combat().unwrap();
        assert_eq!(vm.combat_id, "5457:62");
        assert_eq!(ideo, Some("democratic"));
        // click far away -> no open (after close)
        shell.close();
        assert!(!shell.try_open_at(&cam, 5000.0, 5000.0));
        assert!(!shell.is_open());
    }

    #[test]
    fn close_control_closes_panel() {
        let raster = render::map::ProvinceRaster {
            width: 2,
            height: 1,
            ids: vec![12740, 12740],
            max_id: 12740,
        };
        let cent = ProvinceCentroids::from_raster(&raster);
        let mut shell = CombatShell::new(Some(cent), ProvinceNames::default());
        shell.rebind(&snap(one_combat()));
        let cam = MapCamera::new(200, 100, 2, 1);
        let bubbles = shell.bubbles(&cam);
        shell.try_open_at(&cam, bubbles[0].sx, bubbles[0].sy);
        assert!(shell.is_open());
        // click in the close rect -> closes
        assert!(shell.try_close_at((100.0, 10.0, 20.0, 20.0), 110.0, 20.0));
        assert!(!shell.is_open());
    }

    #[test]
    fn ended_combat_auto_closes_no_stale_target() {
        let raster = render::map::ProvinceRaster {
            width: 2,
            height: 1,
            ids: vec![12740, 12740],
            max_id: 12740,
        };
        let cent = ProvinceCentroids::from_raster(&raster);
        let mut shell = CombatShell::new(Some(cent), ProvinceNames::default());
        shell.rebind(&snap(one_combat()));
        let cam = MapCamera::new(200, 100, 2, 1);
        let bubbles = shell.bubbles(&cam);
        shell.try_open_at(&cam, bubbles[0].sx, bubbles[0].sy);
        assert!(shell.is_open());
        // next tick: the combat ended (empty combats[]) -> panel auto-closes.
        shell.rebind(&snap(json!([])));
        assert!(!shell.is_open());
        assert!(shell.open_combat().is_none());
    }
}
