//! A masked shield as the game draws it: the flag through the shield's own effect
//! and mask, then its overlay through the program the game's code names, each with
//! its authored blend.
use super::*;
use installed_content_core::gui::native::{resources::SHIELD_OVERLAY_EFFECT, sprites::SpriteKind};
use installed_content_image::dds;
use shader_pipeline::artwork::Draw;

/// A `maskedShieldType` resolved for drawing: its flag effect, mask and overlay
/// textures from the content stack, and the flag set its `small` field selects.
#[derive(Clone)]
pub struct MaskedShield {
    name: String,
    small: bool,
    effect: Vec<u8>,
    mask: dds::Texture,
    /// None when the shield declares none or the game cannot load it: the
    /// overlay then has no extent and its pass draws nothing.
    overlay: Option<dds::Texture>,
}

/// The flag's rectangle on the page its pass samples, offset then size: the flag fills a
/// page of its own.
const FLAG_CELL: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

impl MaskedShield {
    /// Refuses, by the sprite's name, a sprite of another kind, a declaration
    /// without an effect or mask, and a mask the game cannot load.
    pub fn load(
        layers: &ResolvedLayers,
        sprite: &installed_content_gfx::gfx::SpriteType,
        decode: Decode,
    ) -> io::Result<Self> {
        let name = &sprite.name;
        let SpriteKind::MaskedShield { shield, .. } = &sprite.facts.kind else {
            return Err(invalid(format!("{name} is not a maskedShieldType")));
        };
        for (field, value) in [
            ("effectFile", &shield.effect_file),
            ("textureFile2 (its mask)", &shield.texture2),
        ] {
            if value.is_empty() {
                return Err(invalid(format!("{name} declares no {field}")));
            }
        }
        let named = |why: io::Error| invalid(format!("{name}: {why}"));
        let overlay = if shield.texture1.is_empty() {
            None
        } else {
            sprite::optional(layers, &shield.texture1, decode).map_err(named)?
        };
        Ok(Self {
            name: name.clone(),
            small: shield.small,
            effect: shield.effect_file.clone(),
            mask: sprite::load(layers, &shield.texture2, decode).map_err(named)?,
            overlay,
        })
    }

    /// Whether the flag comes from the small flag set: the game takes it from
    /// the small atlas.
    pub fn small(&self) -> bool {
        self.small
    }

    /// The overlay's base image, for an app-styled variant ([`Self::with_overlay`]),
    /// or None when the shield draws no overlay.
    pub fn overlay(&self) -> io::Result<Option<Rgba8Image>> {
        self.overlay
            .clone()
            .map(installed_content_image::rgba::dds_base)
            .transpose()
    }

    /// This shield with `overlay` drawn in place of its own: app styling, not
    /// the game's draw.
    pub fn with_overlay(&self, overlay: Rgba8Image) -> Self {
        Self {
            overlay: Some(dds::Texture::from_rgba8(overlay)),
            ..self.clone()
        }
    }

    /// Each flag drawn in the shield, in straight alpha, at the whole-pixel extent
    /// covering its overlay and mask. A refusal names the shield and applies to every flag:
    /// the passes differ only in the flag they sample.
    ///
    /// The game's flag atlas and each flag's cell there are not recorded: the flag is drawn
    /// from a page of its own at [`FLAG_CELL`]. That draws what the game draws only for a
    /// flag program that reads the rectangle solely to map its texture coordinate into the
    /// cell, which is proved from its compiled stages; any other is refused
    /// ([`shader_pipeline::artwork::flag_rectangle_mapped`]).
    pub fn draw(
        &self,
        layers: &ResolvedLayers,
        flags: &[&Rgba8Image],
        decode: Decode,
    ) -> io::Result<Vec<Rgba8Image>> {
        let refuse = |why: String| invalid(format!("shield {}: {why}", self.name));
        shader_pipeline::artwork::flag_rectangle_mapped(layers, &self.effect, decode)
            .map_err(refuse)?;
        let drawables = flags
            .iter()
            .map(|flag| self.passes(&dds::Texture::from_rgba8((*flag).clone())))
            .collect();
        shader_pipeline::artwork::render_layers(layers, drawables, decode).map_err(refuse)
    }

    /// The flag pass, then the overlay pass when the shield has one, in the
    /// drawable's native local coordinates (y up): the overlay over `[0, -1]` to
    /// `[width, height - 1]`, and the mask centred on it by unrounded half
    /// differences, one pixel lower as the game places both. The image's framing
    /// is in the projection alone, keeping the overlay on whole pixels.
    fn passes(&self, flag: &dds::Texture) -> Vec<Draw> {
        let overlay = self.overlay.as_ref().map_or([0.0; 2], |overlay| {
            [overlay.width, overlay.height].map(|edge| edge as f32)
        });
        let mask = [self.mask.width, self.mask.height].map(|edge| edge as f32);
        let half = [0, 1].map(|axis| (overlay[axis] - mask[axis]) * 0.5);
        let overlay_rect = [0.0, -1.0, overlay[0], overlay[1]];
        let flag_rect = [half[0], half[1] - 1.0, mask[0], mask[1]];
        let low = [0, 1].map(|axis| {
            let rects = [overlay_rect, flag_rect];
            rects
                .iter()
                .map(|rect| rect[axis])
                .fold(f32::INFINITY, f32::min)
                .floor()
        });
        let high = [0, 1].map(|axis| {
            [overlay_rect, flag_rect]
                .iter()
                .map(|rect| rect[axis] + rect[axis + 2])
                .fold(f32::NEG_INFINITY, f32::max)
                .ceil()
        });
        let size = [0, 1].map(|axis| (high[axis] - low[axis]) as u32);
        let matrix = projection(size, low);
        // The flag pass writes all 96 bytes of b0: the matrix, the flag's atlas
        // rectangle and a zero tail. It binds the vertex stage only.
        let mut flag_constants = matrix.to_vec();
        flag_constants.extend(FLAG_CELL);
        flag_constants.extend([0.0; 4]);
        let mut passes = vec![Draw {
            effect: self.effect.clone(),
            defines: Vec::new(),
            size,
            vertices: quad(flag_rect),
            constants: vec![(0, flag_constants, vec![0..96])],
            pixel_constants: false,
            textures: vec![(0, Some(flag.clone())), (1, Some(self.mask.clone()))],
        }];
        if let Some(texture) = &self.overlay {
            // The overlay writes 104 of b0's 112 bytes: the matrix, the button's
            // white colour, zero frame offsets and zero Time and AnimationTime.
            let mut overlay_constants = matrix.to_vec();
            overlay_constants.extend([1.0; 4]);
            overlay_constants.resize(28, 0.0);
            passes.push(Draw {
                effect: SHIELD_OVERLAY_EFFECT.as_bytes().to_vec(),
                defines: Vec::new(),
                size,
                vertices: quad(overlay_rect),
                constants: vec![(0, overlay_constants, vec![0..104])],
                pixel_constants: true,
                // The mask's binding from the flag pass is retained.
                textures: vec![(0, Some(texture.clone())), (1, Some(self.mask.clone()))],
            });
        }
        passes
    }
}

/// The target's projection, its bottom-left at local `low` (native y up), in
/// column-major storage; geometry keeps local z = 1 and the drawable's
/// translation z = -1, as the game places it.
fn projection(size: [u32; 2], low: [f32; 2]) -> [f32; 16] {
    let [w, h] = size.map(|edge| edge as f32);
    [
        2.0 / w,
        0.0,
        0.0,
        0.0,
        0.0,
        2.0 / h,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
        0.0,
        -1.0 - 2.0 * low[0] / w,
        -1.0 - 2.0 * low[1] / h,
        -1.0,
        1.0,
    ]
}

/// A quad over local `[x, y, width, height]` with the whole texture:
/// bottom-left, top-left, bottom-right, top-right.
fn quad([x, y, w, h]: [f32; 4]) -> [[f32; 5]; 4] {
    [
        [x, y, 1.0, 0.0, 1.0],
        [x, y + h, 1.0, 0.0, 0.0],
        [x + w, y, 1.0, 1.0, 1.0],
        [x + w, y + h, 1.0, 1.0, 0.0],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shield(overlay: Option<[u32; 2]>, mask: [u32; 2]) -> MaskedShield {
        let texture = |[width, height]: [u32; 2]| {
            dds::Texture::from_rgba8(Rgba8Image {
                width,
                height,
                pixels: vec![255; (width * height * 4) as usize],
            })
        };
        MaskedShield {
            name: "GFX_fixture_shield".into(),
            small: false,
            effect: b"gfx/FX/fixture_flag.lua".to_vec(),
            mask: texture(mask),
            overlay: overlay.map(texture),
        }
    }

    fn flag() -> dds::Texture {
        dds::Texture::from_rgba8(Rgba8Image {
            width: 2,
            height: 1,
            pixels: vec![9; 8],
        })
    }

    #[test]
    fn the_passes_keep_native_local_coordinates_and_the_flag_binds_the_vertex_stage_only() {
        let passes = shield(Some([8, 6]), [3, 4]).passes(&flag());
        let [flag_pass, overlay] = &passes[..] else {
            panic!("two passes")
        };
        assert_eq!(flag_pass.effect, b"gfx/FX/fixture_flag.lua");
        assert_eq!(overlay.effect, SHIELD_OVERLAY_EFFECT.as_bytes());
        assert_eq!([flag_pass.size, overlay.size], [[8, 6]; 2]);
        // The overlay over (0, -1) to (8, 5); the mask an unrounded half
        // difference in, 2.5 across and 1 up, one pixel lower.
        assert_eq!(flag_pass.vertices[0][..2], [2.5, 0.0]);
        assert_eq!(flag_pass.vertices[3][..2], [5.5, 4.0]);
        assert_eq!(overlay.vertices[0][..2], [0.0, -1.0]);
        assert_eq!(overlay.vertices[3][..2], [8.0, 5.0]);
        // One drawable transform: only the projection frames the image.
        let c0 = &flag_pass.constants[0].1;
        assert_eq!(c0.len(), 24);
        assert_eq!(c0[..16], overlay.constants[0].1[..16]);
        assert_eq!([c0[12], c0[13], c0[14]], [-1.0, -1.0 + 2.0 / 6.0, -1.0]);
        assert_eq!(&c0[16..], [0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(flag_pass.constants[0].2, [0..96]);
        assert!(!flag_pass.pixel_constants);
        assert_eq!(flag_pass.textures.len(), 2);
        let c0 = &overlay.constants[0].1;
        assert_eq!(
            &c0[16..],
            [1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
        );
        assert_eq!(overlay.constants[0].2, [0..104]);
        assert!(overlay.pixel_constants);
    }

    #[test]
    fn a_mask_wider_than_its_overlay_widens_the_image_and_keeps_the_overlay_on_whole_pixels() {
        let passes = shield(Some([4, 4]), [7, 4]).passes(&flag());
        // The mask runs from -1.5 to 5.5: the image starts two pixels left of the
        // overlay and ends at 6.
        assert_eq!(passes[0].size, [8, 4]);
        assert_eq!(passes[0].vertices[0][0], -1.5);
        assert_eq!(passes[0].constants[0].1[12], -1.0 + 2.0 * 2.0 / 8.0);
    }

    #[test]
    fn a_shield_without_an_overlay_frames_the_flag_pass_alone() {
        let passes = shield(None, [4, 2]).passes(&flag());
        assert_eq!(passes.len(), 1);
        // The mask centred on the empty overlay at (0, -1).
        assert_eq!(passes[0].size, [4, 2]);
        assert_eq!(passes[0].vertices[0][..2], [-2.0, -2.0]);
        assert_eq!(passes[0].vertices[3][..2], [2.0, 0.0]);
    }

    #[test]
    #[ignore = "requires DXC and a GPU; synthetic content only"]
    fn shields_composite_the_flag_and_overlay_passes_with_their_authored_blend() {
        use super::super::texture_tests::{
            from_url, png_bytes, shield_flag_program, shield_programs,
        };
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let gfx = "spriteTypes = {
            maskedShieldType = { name = GFX_fixture_shield textureFile1 = gfx/overlay.png textureFile2 = gfx/mask.png effectFile = gfx/FX/shield_flag.lua }
            maskedShieldType = { name = GFX_additive_shield textureFile1 = gfx/overlay.png textureFile2 = gfx/mask.png effectFile = gfx/FX/additive_flag.lua }
            maskedShieldType = { name = GFX_bare_shield textureFile2 = gfx/mask.png effectFile = gfx/FX/shield_flag.lua }
            maskedShieldType = { name = GFX_atlas_shield textureFile1 = gfx/overlay.png textureFile2 = gfx/mask.png effectFile = gfx/FX/atlas_flag.lua }
        }";
        let mut programs = shield_programs();
        programs.push((
            "gfx/FX/additive_flag.shader",
            shield_flag_program("SourceBlend = \"ONE\" DestBlend = \"ONE\""),
        ));
        // A program that samples the flag at a point its rectangle's size sets.
        let atlas =
            shield_flag_program("SourceBlend = \"SRC_ALPHA\" DestBlend = \"INV_SRC_ALPHA\"")
                .replace(
                    "FlagCoords.xy + i.uv * FlagCoords.zw",
                    "FlagCoords.zw * 0.75",
                );
        programs.push(("gfx/FX/atlas_flag.shader", atlas));
        let mut files: Vec<_> = programs
            .iter()
            .map(|(path, text)| (*path, text.as_str()))
            .collect();
        files.push(("interface/shield.gfx", gfx));
        let (catalogue, folder) = super::super::tests::folder_catalogue(&files);
        let image = |width, height, pixel: &dyn Fn(u32) -> [u8; 4]| Rgba8Image {
            width,
            height,
            pixels: (0..width * height)
                .flat_map(|at| pixel(at % width))
                .collect(),
        };
        // The mask's left half is opaque; the overlay is half-transparent black.
        let mask = image(4, 2, &|x| [255, 255, 255, if x < 2 { 255 } else { 0 }]);
        for (path, texture) in [
            ("gfx/mask.png", mask),
            ("gfx/overlay.png", image(6, 4, &|_| [0, 0, 0, 128])),
        ] {
            std::fs::write(folder.0.join(path), png_bytes(&texture)).unwrap();
        }
        let layers = &catalogue.content.layers;
        let load = |name| {
            MaskedShield::load(
                layers,
                catalogue.sprites.lookup(name).unwrap(),
                Decode::default(),
            )
            .unwrap()
        };
        // Red on the left, blue on the right; the mask shows only the red half.
        let flag = image(2, 1, &|x| {
            if x == 0 {
                [200, 0, 0, 90]
            } else {
                [0, 0, 200, 90]
            }
        });
        let drawn = load("GFX_fixture_shield")
            .draw(layers, &[&flag, &flag], Decode::default())
            .unwrap();
        assert_eq!(drawn.len(), 2);
        let drawn = from_url(&png_url(&drawn[0]).unwrap());
        assert_eq!([drawn.width, drawn.height], [6, 4]);
        for y in 0..4 {
            for x in 0..6 {
                let at = ((y * 6 + x) * 4) as usize;
                let pixel = &drawn.pixels[at..at + 4];
                // Inside the mask's opaque half, half-black over the flag's colour at
                // the mask's alpha; elsewhere only the overlay.
                let expected = if (1..3).contains(&x) && (1..3).contains(&y) {
                    [100, 0, 0, 255]
                } else {
                    [0, 0, 0, 128]
                };
                assert!(
                    pixel.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 1),
                    "({x}, {y}): {pixel:?}"
                );
            }
        }
        let why = load("GFX_additive_shield")
            .draw(layers, &[&flag], Decode::default())
            .unwrap_err()
            .to_string();
        assert!(
            why.contains("GFX_additive_shield")
                && why.contains("additive_flag")
                && why.contains("only a source-over pass"),
            "{why}"
        );
        // Without an overlay the flag pass alone is framed: the mask's extent.
        let bare = load("GFX_bare_shield")
            .draw(layers, &[&flag], Decode::default())
            .unwrap();
        let bare = from_url(&png_url(&bare[0]).unwrap());
        assert_eq!([bare.width, bare.height], [4, 2]);
        assert_eq!(&bare.pixels[..4], [200, 0, 0, 255]);
        assert_eq!(bare.pixels[3 * 4 + 3], 0);
        // A program whose output depends on the flag's atlas rectangle is refused.
        let why = load("GFX_atlas_shield")
            .draw(layers, &[&flag], Decode::default())
            .unwrap_err()
            .to_string();
        assert!(
            why.contains("GFX_atlas_shield")
                && why.contains("atlas_flag")
                && why.contains("rectangle"),
            "{why}"
        );
    }

    /// The installed flag program reads its rectangle only to map its coordinate.
    #[test]
    #[ignore = "requires DXC and a GPU; reads the installation"]
    fn the_installed_flag_program_maps_its_coordinate_through_the_rectangle_alone() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let game = std::path::PathBuf::from(std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR")).unwrap());
        let layers = ResolvedLayers::from_folders(&[(game.as_path(), &[][..])]).unwrap();
        let sprites = installed_content_gfx::gfx::read(&layers).unwrap();
        let SpriteKind::MaskedShield { shield, .. } =
            &sprites.lookup("GFX_shield_medium").unwrap().facts.kind
        else {
            panic!("the installed top-bar shield is a masked shield")
        };
        shader_pipeline::artwork::flag_rectangle_mapped(
            &layers,
            &shield.effect_file,
            Decode::default(),
        )
        .unwrap();
    }

    #[test]
    fn shields_refuse_other_kinds_and_missing_fields_by_name_and_draw_without_an_overlay() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (catalogue, folder) = super::super::tests::folder_catalogue(&[(
            "interface/shield.gfx",
            "spriteTypes = {
                spriteType = { name = GFX_plain textureFile = gfx/a.png }
                maskedShieldType = { name = GFX_no_effect textureFile1 = gfx/a.png textureFile2 = gfx/a.png }
                maskedShieldType = { name = GFX_no_mask textureFile1 = gfx/a.png effectFile = gfx/FX/a.lua }
                maskedShieldType = { name = GFX_no_overlay textureFile2 = gfx/a.png effectFile = gfx/FX/a.lua }
                maskedShieldType = { name = GFX_lost_overlay textureFile1 = gfx/absent.png textureFile2 = gfx/a.png effectFile = gfx/FX/a.lua }
                maskedShieldType = { name = GFX_absent textureFile1 = gfx/absent.png textureFile2 = gfx/absent.png effectFile = gfx/FX/a.lua }
            }",
        )]);
        for (name, expected) in [
            ("GFX_plain", "GFX_plain is not a maskedShieldType"),
            ("GFX_no_effect", "GFX_no_effect declares no effectFile"),
            ("GFX_no_mask", "GFX_no_mask declares no textureFile2"),
            (
                "GFX_absent",
                "GFX_absent: GUI texture gfx/absent.png is unavailable",
            ),
        ] {
            let why = MaskedShield::load(
                &catalogue.content.layers,
                catalogue.sprites.lookup(name).unwrap(),
                Decode::default(),
            )
            .err()
            .unwrap()
            .to_string();
            assert!(why.contains(expected), "{name}: {why}");
        }
        // An overlay that is not declared, or that the game cannot load, has no
        // extent: the shield draws its flag pass alone.
        let mask = Rgba8Image {
            width: 2,
            height: 2,
            pixels: vec![255; 16],
        };
        std::fs::write(
            folder.0.join("gfx/a.png"),
            super::super::texture_tests::png_bytes(&mask),
        )
        .unwrap();
        for name in ["GFX_no_overlay", "GFX_lost_overlay"] {
            let shield = MaskedShield::load(
                &catalogue.content.layers,
                catalogue.sprites.lookup(name).unwrap(),
                Decode::default(),
            )
            .unwrap();
            assert!(shield.overlay().unwrap().is_none(), "{name}");
        }
    }
}
