//! A laid-out window as the browser draws it: textured quads in the window's interface
//! pixels, in the game's draw order, with their clips.
//!
//! The native layout ([`viewer_foundation::map::native_gui`]) places every part. This module
//! only flattens each part into the quads its draw submits, with the same corners, texture
//! coordinates and colours the map viewer submits, so no layout rule is evaluated twice. The
//! browser samples each quad's texture page at its native size and multiplies it by the
//! quad's colour. It does not execute the part's effect program: every effect technique drawn
//! that way is named and counted in [`Scene::effects`].

use std::collections::BTreeMap;

use serde::Serialize;
use ts_rs::TS;
use viewer_foundation::map::{
    native_gui::{InlineSource, Part, PartKind, Sheets, lossy},
    sprites::NO_TEXTURE,
};

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
    /// Every texture page a quad samples, by its id in the window's artwork session.
    pub textures: Vec<SceneTexture>,
    /// Whole-pixel clips `x0, y0, x1, y1` in window pixels, which quads name by index.
    pub clips: Vec<[i32; 4]>,
    /// Every quad, in draw order.
    pub quads: Vec<SceneQuad>,
    /// Areas the pointer can rest on, in draw order, with what the app shows for them.
    pub hovers: Vec<SceneHover>,
    /// Lists the pointer can scroll, by their element path.
    pub lists: Vec<SceneList>,
    /// The effect techniques drawn as their texture times their colour, counted.
    pub effects: Vec<String>,
    /// What the game draws nothing for in this window, or that is drawn incompletely, counted.
    pub notes: Vec<String>,
}

/// A texture page as loaded at its native size.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SceneTexture {
    pub id: u32,
    pub width: u32,
    pub height: u32,
}

/// One textured quad: corners top-left, top-right, bottom-right, bottom-left in window pixels,
/// each with its texture coordinate (normalised, possibly beyond 0..1 where the texture
/// repeats), and a straight RGBA multiplier.
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SceneQuad {
    pub texture: u32,
    pub corners: [[f32; 2]; 4],
    pub uv: [[f32; 2]; 4],
    pub color: [f32; 4],
    /// Index into [`Scene::clips`].
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clip: Option<u32>,
    /// A glyph of bitmap text: the page's alpha is the glyph's coverage.
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glyph: Option<bool>,
    /// Part of a row the app mutes, as it mutes table rows that cannot be acted on: the game
    /// marks such a row with its controls' disabled effect, which the browser does not run.
    /// Consecutive muted quads are muted together.
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub muted: Option<bool>,
}

/// An area the app explains on hover: an in-game name with its details, codes included.
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SceneHover {
    /// `x, y, width, height` in window pixels.
    pub rect: [f32; 4],
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clip: Option<u32>,
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

    pub fn lines(self) -> Vec<String> {
        let mut lines: Vec<_> = self.0.into_iter().collect();
        lines.sort_by_key(|(_, (order, _))| *order);
        lines
            .into_iter()
            .map(|(reason, (_, count))| format!("{reason} (count: {count})"))
            .collect()
    }
}

/// Flattens composed parts into [`Scene::quads`], their textures and clips. A part the
/// browser cannot draw as the game does (a progress bar, a fan, a masked or animated sprite, a
/// baked text sprite) is left out and noted by kind and path: the scene is then incomplete.
pub struct Flatten<'a> {
    sheets: &'a dyn Sheets,
    /// A loaded texture page's size, by its id.
    sizes: &'a dyn Fn(usize) -> Option<[u32; 2]>,
    pub quads: Vec<SceneQuad>,
    textures: BTreeMap<u32, SceneTexture>,
    clips: Vec<[i32; 4]>,
    pub effects: Counted,
    pub notes: Counted,
    /// Parts left out: the scene does not show everything the game draws.
    pub omitted: usize,
    /// Whether the part being flattened is muted.
    muted: bool,
}

impl<'a> Flatten<'a> {
    pub fn new(sheets: &'a dyn Sheets, sizes: &'a dyn Fn(usize) -> Option<[u32; 2]>) -> Self {
        Self {
            sheets,
            sizes,
            quads: Vec::new(),
            textures: BTreeMap::new(),
            clips: Vec::new(),
            effects: Counted::default(),
            notes: Counted::default(),
            omitted: 0,
            muted: false,
        }
    }

    pub fn clip(&mut self, clip: Option<[i32; 4]>) -> Option<u32> {
        let clip = clip?;
        Some(match self.clips.iter().position(|kept| *kept == clip) {
            Some(index) => index as u32,
            None => {
                self.clips.push(clip);
                (self.clips.len() - 1) as u32
            }
        })
    }

    fn texture(&mut self, id: usize) -> Option<u32> {
        if id == NO_TEXTURE {
            return None;
        }
        let [width, height] = (self.sizes)(id)?;
        let id = u32::try_from(id).ok()?;
        self.textures
            .entry(id)
            .or_insert(SceneTexture { id, width, height });
        Some(id)
    }

    fn omit(&mut self, part: &Part, why: &str) {
        self.omitted += 1;
        self.notes
            .note(format!("{} is not drawn: {why}", part.path));
    }

    fn effect(&mut self, part: &Part) {
        if let Some(compiled) = &part.shading.compiled {
            self.effects.note(format!(
                "{} technique {}",
                compiled.asset.source.effect, compiled.technique
            ));
        }
    }

    fn push(
        &mut self,
        part: &Part,
        texture: usize,
        corners: [[f32; 2]; 4],
        uv: [[f32; 2]; 4],
        color: [f32; 4],
        glyph: bool,
    ) {
        let Some(texture) = self.texture(texture) else {
            // The null texture samples zero: the quad shows nothing under alpha blending.
            self.notes.note(format!(
                "{} draws nothing visible: its texture did not load",
                part.path
            ));
            return;
        };
        let clip = self.clip(part.clip);
        self.quads.push(SceneQuad {
            texture,
            corners,
            uv,
            color,
            clip,
            glyph: glyph.then_some(true),
            muted: self.muted.then_some(true),
        });
    }

    /// The quads one part submits, as the map viewer's draw submits them, muted or not.
    pub fn part(&mut self, part: &Part, muted: bool) {
        self.muted = muted;
        let corners_uv = |[u0, v0, u1, v1]: [f32; 4]| [[u0, v0], [u1, v0], [u1, v1], [u0, v1]];
        let rect_corners =
            |[x, y, w, h]: [f32; 4]| [[x, y], [x + w, y], [x + w, y + h], [x, y + h]];
        // A text's projection scales its position about the viewport's top-left.
        let project =
            |corners: [[f32; 2]; 4], [fx, fy]: [f32; 2]| corners.map(|[x, y]| [x * fx, y * fy]);
        let sheets = self.sheets;
        let sheet = |sprite: &[u8]| sheets.sheet(sprite).map(|sheet| sheet.texture);
        match &part.kind {
            PartKind::Sprite {
                sprite,
                geometry,
                uv,
                offsets,
                masked,
            } => {
                if *masked {
                    return self.omit(part, "masked sprites are not drawn in the dashboard yet");
                }
                let Some(texture) = sheet(sprite) else {
                    return self.omit(part, &format!("sprite {} is not loaded", lossy(sprite)));
                };
                self.effect(part);
                // The draw passes the frame's place in the strip as its `Offset`.
                let [u0, v0, u1, v1] = *uv;
                let uv = [
                    u0 + offsets[0],
                    v0 + offsets[1],
                    u1 + offsets[0],
                    v1 + offsets[1],
                ];
                self.push(
                    part,
                    texture,
                    geometry.corners(),
                    corners_uv(uv),
                    part.color,
                    false,
                );
            }
            PartKind::Cell {
                texture,
                masking,
                uv,
                geometry,
            } => {
                if masking.is_some() {
                    return self.omit(part, "masked cells are not drawn in the dashboard yet");
                }
                self.effect(part);
                self.push(
                    part,
                    *texture,
                    geometry.corners(),
                    corners_uv(*uv),
                    part.color,
                    false,
                );
            }
            PartKind::Tiles { sprite, quads, .. } => {
                let Some(texture) = sheet(sprite) else {
                    return self.omit(part, &format!("sprite {} is not loaded", lossy(sprite)));
                };
                self.effect(part);
                for (rect, uv) in quads {
                    self.push(
                        part,
                        texture,
                        rect_corners(*rect),
                        corners_uv(*uv),
                        part.color,
                        false,
                    );
                }
            }
            PartKind::Inline {
                source,
                rect,
                uv,
                modulate,
                projection,
                ..
            } => {
                let texture = match source {
                    InlineSource::Sprite(sprite) => match sheet(sprite) {
                        Some(texture) => texture,
                        None => {
                            return self
                                .omit(part, &format!("sprite {} is not loaded", lossy(sprite)));
                        }
                    },
                    InlineSource::Texture(texture) => *texture,
                };
                self.effect(part);
                let color = multiply(part.color, *modulate);
                self.push(
                    part,
                    texture,
                    project(rect_corners(*rect), *projection),
                    corners_uv(*uv),
                    color,
                    false,
                );
            }
            PartKind::Text {
                glyphs,
                scale,
                modulate,
                projection,
                ..
            } => {
                self.effect(part);
                for glyph in glyphs {
                    let [x, y] = glyph.at;
                    let [w, h] = glyph.size.map(|value| value as f32 * scale);
                    self.push(
                        part,
                        glyph.texture,
                        project(rect_corners([x, y, w, h]), *projection),
                        corners_uv(glyph.uv),
                        multiply(glyph.color, *modulate),
                        true,
                    );
                }
            }
            PartKind::Progress { .. } => {
                self.omit(part, "progress bars are not drawn in the dashboard yet")
            }
            PartKind::Fan { .. } => self.omit(
                part,
                "circular progress bars are not drawn in the dashboard yet",
            ),
            PartKind::MaskedFlag { .. } => {
                self.omit(part, "masked flags are not drawn in the dashboard yet")
            }
            PartKind::Animated(_) => self.omit(
                part,
                "sprites with animation records are not drawn in the dashboard yet",
            ),
            PartKind::TextSprite { .. } => self.omit(
                part,
                "baked text sprites are not drawn in the dashboard yet",
            ),
        }
    }

    /// The textures and clips the quads name.
    #[allow(clippy::type_complexity)]
    pub fn finish(
        self,
    ) -> (
        Vec<SceneQuad>,
        Vec<SceneTexture>,
        Vec<[i32; 4]>,
        Counted,
        Counted,
        usize,
    ) {
        (
            self.quads,
            self.textures.into_values().collect(),
            self.clips,
            self.effects,
            self.notes,
            self.omitted,
        )
    }
}

fn multiply(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2], a[3] * b[3]]
}
