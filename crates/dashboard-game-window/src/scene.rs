//! A laid-out window as the game draws it: one image of the window in its interface pixels,
//! painted offscreen through the native GUI draw ([`crate::paint`]), with the areas the app
//! explains on hover and the lists the pointer scrolls.
//!
//! The image carries every part's own effect, technique, blend and sampling, text effects and
//! colour runs. The browser shows it as it is and only adds its own chrome around it.

use std::collections::BTreeMap;

use serde::Serialize;
use ts_rs::TS;

/// One drawn game window.
#[derive(Clone, Debug, Default, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Scene {
    /// The root template's name.
    pub name: String,
    /// The window's own size in interface pixels: what the browser fits, never re-flows.
    pub size: [i32; 2],
    /// The interface viewport the window was read and laid out in.
    pub viewport: [i32; 2],
    /// The window as the game draws it, a straight-alpha PNG data URL of `size`.
    pub image: String,
    /// Areas the pointer can rest on, in draw order, with what the app shows for them.
    pub hovers: Vec<SceneHover>,
    /// Lists the pointer can scroll, by their element path.
    pub lists: Vec<SceneList>,
    /// What the game draws nothing for in this window, or that is drawn incompletely, counted.
    pub notes: Vec<String>,
}

/// An area the app explains on hover: an in-game name with its details, codes included.
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SceneHover {
    /// `x, y, width, height` in window pixels.
    pub rect: [f32; 4],
    /// The whole-pixel clip `x0, y0, x1, y1` the area is drawn within, in window pixels.
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clip: Option<[i32; 4]>,
    /// The list the area is a row of, by its path ([`SceneList::path`]): focusing it scrolls
    /// that list to it.
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list: Option<String>,
    pub title: String,
    pub details: Vec<String>,
}

/// A list the browser scrolls: the pointer over `rect` moves its offset between 0 and `max`
/// of the list's own content units, and the window is laid out again at that offset. `rect`
/// and its rows' areas are window pixels: a distance there is `scale` times the same distance
/// in offset units.
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SceneList {
    pub path: String,
    pub rect: [i32; 4],
    pub offset: i32,
    pub max: i32,
    /// Whether it scrolls across, its rows side by side; otherwise down.
    pub horizontal: bool,
    /// Window pixels per offset unit: the scale of the container holding the list's rows.
    pub scale: f32,
}

/// Counted reasons, in the order first noted.
#[derive(Default)]
pub struct Counted(BTreeMap<String, (usize, usize)>);

impl Counted {
    pub fn note(&mut self, reason: impl Into<String>) {
        let next = self.0.len();
        self.0.entry(reason.into()).or_insert((next, 0)).1 += 1;
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn lines(self) -> Vec<String> {
        let mut lines: Vec<_> = self.0.into_iter().collect();
        lines.sort_by_key(|(_, (order, _))| *order);
        lines
            .into_iter()
            .map(|(reason, (_, count))| format!("{reason} (count: {count})"))
            .collect()
    }
}
