//! World text draw data shared by name and ribbon producers.
/// One placed name in world space, with its glyph quads already converted.
#[derive(Clone, Debug)]
pub struct WorldName {
    /// The country tag, for reporting.
    pub key: String,
    /// The resolved, un-upper-cased display string.
    pub text: String,
    /// Font page texture id each glyph samples.
    pub pages: Vec<usize>,
    /// Four world corners per glyph, top-left, top-right, bottom-right,
    /// bottom-left in the glyph's own frame.
    pub corners: Vec<[f32; 3]>,
    /// One `[u0, v0, u1, v1]` per glyph, matching `corners`.
    pub uvs: Vec<[f32; 4]>,
    /// `[min x, min z, max x, max z]` in world units.
    pub aabb: [f32; 4],
    /// True when the area crossed the date line and a second copy shifted by
    /// `-MAP_SIZE_X` has to be drawn.
    pub wrapped: bool,
    /// `scale * FONT_SIZE_CONV`, the engine's font-builder argument.
    pub font_size: f32,
    /// Spaces the height-limited branch inserted between every pair.
    pub spaces: usize,
    /// This name's `Transp_OffsetX.x` factor: one for a country name, arrow-colour alpha for a
    /// battle-plan label.
    pub alpha: f32,
}

impl WorldName {
    /// A battle-plan label: one flat quad per glyph, corners top-left,
    /// top-right, bottom-right, bottom-left, drawn at `alpha`.
    pub fn from_glyphs(
        key: String,
        text: String,
        glyphs: impl IntoIterator<Item = ([[f32; 3]; 4], [f32; 4], usize)>,
        alpha: f32,
    ) -> Option<Self> {
        let mut corners = Vec::new();
        let mut uvs = Vec::new();
        let mut pages = Vec::new();
        let mut aabb = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for (quad, uv, page) in glyphs {
            for corner in quad {
                aabb[0] = aabb[0].min(corner[0]);
                aabb[1] = aabb[1].min(corner[2]);
                aabb[2] = aabb[2].max(corner[0]);
                aabb[3] = aabb[3].max(corner[2]);
                corners.push(corner);
            }
            uvs.push(uv);
            pages.push(page);
        }
        (!uvs.is_empty()).then_some(Self {
            key,
            text,
            pages,
            corners,
            uvs,
            aabb,
            wrapped: false,
            font_size: 0.0,
            spaces: 0,
            alpha,
        })
    }

    pub fn glyph_count(&self) -> usize {
        self.uvs.len()
    }

    /// One line naming the area, its string and the fit the engine produced.
    pub fn summary(&self) -> String {
        format!(
            "{}: {:?} font size {:.2}, {} spaces between characters, {} glyphs,              world x {:.0}..{:.0} z {:.0}..{:.0}",
            self.key,
            self.text,
            self.font_size,
            self.spaces,
            self.glyph_count(),
            self.aabb[0],
            self.aabb[2],
            self.aabb[1],
            self.aabb[3]
        )
    }
}

/// The country names placed for one hour.
#[derive(Clone, Debug, Default)]
pub struct Placements {
    pub country: Vec<WorldName>,
    /// What could not be placed, stated rather than silently dropped.
    pub warnings: Vec<String>,
    /// Which placement this is: the name pass builds its static vertices once
    /// per generation. Zero is the empty default; every name placement is unique.
    pub generation: u64,
}

/// The next [`Placements::generation`].
pub fn next_generation() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}
