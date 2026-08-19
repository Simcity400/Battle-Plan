
use crate::{load_dds_rgba_mips, BorderMaterial, GameFileResolver, Result, RgbaImage};

pub const BORDER_TILE: f32 = 0.4;

#[derive(Clone, Debug, PartialEq)]
pub struct BorderTextureProfile {
    pub material: BorderMaterial,
    pub source: String,
    pub width: usize,
    pub height: usize,
    pub cross_section: Vec<[u8; 4]>,
    pub peak_alpha: u8,
    pub peak_rgb: [u8; 3],
    pub ink_first_row: usize,
    pub ink_last_row: usize,
}

pub const INK_ALPHA_THRESHOLD: u8 = 10;

impl BorderTextureProfile {
    pub fn ink_rows(&self) -> usize {
        self.ink_last_row.saturating_sub(self.ink_first_row) + 1
    }

    pub fn ink_fraction_of_quad(&self) -> f32 {
        if self.height == 0 {
            return 0.0;
        }
        self.ink_rows() as f32 / self.height as f32
    }

    pub fn ink_rgba(&self) -> [u8; 4] {
        [
            self.peak_rgb[0],
            self.peak_rgb[1],
            self.peak_rgb[2],
            self.peak_alpha,
        ]
    }
}

pub(crate) fn decode_border_texture_profile(
    material: BorderMaterial,
    source: &str,
    top_mip: &RgbaImage,
) -> Result<BorderTextureProfile> {
    if top_mip.width == 0 || top_mip.height == 0 {
        return Err(format!(
            "border texture law blocked: {source} decoded to a {}x{} image",
            top_mip.width, top_mip.height
        )
        .into());
    }
    let expected = top_mip.width * top_mip.height * 4;
    if top_mip.pixels.len() != expected {
        return Err(format!(
            "border texture law blocked: {source} has {} bytes for {}x{} RGBA (expected {expected})",
            top_mip.pixels.len(),
            top_mip.width,
            top_mip.height
        )
        .into());
    }
    let mid_col = top_mip.width / 2;
    let mut cross_section = Vec::with_capacity(top_mip.height);
    for row in 0..top_mip.height {
        let idx = (row * top_mip.width + mid_col) * 4;
        cross_section.push([
            top_mip.pixels[idx],
            top_mip.pixels[idx + 1],
            top_mip.pixels[idx + 2],
            top_mip.pixels[idx + 3],
        ]);
    }
    let peak_alpha = cross_section.iter().map(|p| p[3]).max().unwrap_or(0);
    let peak_rgb = cross_section
        .iter()
        .find(|p| p[3] == peak_alpha)
        .map(|p| [p[0], p[1], p[2]])
        .unwrap_or([0, 0, 0]);
    let ink_rows: Vec<usize> = cross_section
        .iter()
        .enumerate()
        .filter(|(_, p)| p[3] > INK_ALPHA_THRESHOLD)
        .map(|(row, _)| row)
        .collect();
    let (ink_first_row, ink_last_row) = match (ink_rows.first(), ink_rows.last()) {
        (Some(&first), Some(&last)) => (first, last),
        _ => {
            return Err(format!(
                "border texture law blocked: {source} cross-section has no ink row above alpha \
                 {INK_ALPHA_THRESHOLD}; a border DDS must have a visible stripe"
            )
            .into());
        }
    };
    Ok(BorderTextureProfile {
        material,
        source: source.to_string(),
        width: top_mip.width,
        height: top_mip.height,
        cross_section,
        peak_alpha,
        peak_rgb,
        ink_first_row,
        ink_last_row,
    })
}

pub fn border_texture_profile_from_resolver(
    resolver: &GameFileResolver,
    material: BorderMaterial,
) -> Result<BorderTextureProfile> {
    let source = crate::border_diffuse_source(material);
    let path = resolver
        .resolve_required(source, "border diffuse DDS")
        .map_err(|err| format!("border texture law blocked: {err}"))?;
    let mips = load_dds_rgba_mips(&path)
        .map_err(|err| format!("border texture law blocked: failed to decode {source}: {err}"))?;
    let top = mips
        .first()
        .ok_or_else(|| format!("border texture law blocked: {source} decoded zero mips"))?;
    decode_border_texture_profile(material, source, top)
}

pub fn all_border_texture_profiles_from_resolver(
    resolver: &GameFileResolver,
) -> Result<Vec<BorderTextureProfile>> {
    [
        BorderMaterial::Province,
        BorderMaterial::State,
        BorderMaterial::Country,
        BorderMaterial::SeaCoast,
        BorderMaterial::SeaRegion,
        BorderMaterial::Impassable,
    ]
    .into_iter()
    .map(|material| border_texture_profile_from_resolver(resolver, material))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image_from_alpha_column(width: usize, height: usize, column: &[(u8, [u8; 3])]) -> RgbaImage {
        assert_eq!(column.len(), height);
        let mut pixels = vec![0u8; width * height * 4];
        for (row, (alpha, rgb)) in column.iter().enumerate() {
            for col in 0..width {
                let idx = (row * width + col) * 4;
                pixels[idx] = rgb[0];
                pixels[idx + 1] = rgb[1];
                pixels[idx + 2] = rgb[2];
                pixels[idx + 3] = *alpha;
            }
        }
        RgbaImage {
            width,
            height,
            pixels,
        }
    }

    #[test]
    fn decodes_centered_soft_bell_as_ink_band() {
        let column = [
            (0u8, [0u8; 3]),
            (0, [0; 3]),
            (33, [0; 3]),
            (96, [0; 3]),
            (114, [0; 3]),
            (114, [0; 3]),
            (96, [0; 3]),
            (33, [0; 3]),
            (0, [0; 3]),
            (0, [0; 3]),
        ];
        let img = image_from_alpha_column(8, 10, &column);
        let p =
            decode_border_texture_profile(BorderMaterial::SeaCoast, "test/sea.dds", &img).unwrap();
        assert_eq!(p.peak_alpha, 114);
        assert_eq!(p.peak_rgb, [0, 0, 0], "sea ink is black, never red");
        assert_eq!(p.ink_first_row, 2);
        assert_eq!(p.ink_last_row, 7);
        assert_eq!(p.ink_rows(), 6);
        let centre = (p.ink_first_row + p.ink_last_row) as f32 / 2.0;
        assert!((centre - (p.height as f32 - 1.0) / 2.0).abs() <= 1.0);
    }

    #[test]
    fn thin_black_plateau_is_thin_and_black_like_province() {
        let mut column = vec![(0u8, [0u8; 3]); 32];
        for row in 14..=17 {
            column[row] = (179, [0, 0, 0]);
        }
        column[13] = (4, [0, 0, 0]);
        column[18] = (4, [0, 0, 0]);
        let img = image_from_alpha_column(64, 32, &column);
        let p =
            decode_border_texture_profile(BorderMaterial::Province, "test/prov.dds", &img).unwrap();
        assert_eq!(p.peak_alpha, 179);
        assert_eq!(p.peak_rgb, [0, 0, 0]);
        assert_eq!(p.ink_first_row, 14);
        assert_eq!(p.ink_last_row, 17);
        assert_eq!(p.ink_rows(), 4);
        assert!((p.ink_fraction_of_quad() - 0.125).abs() < 0.01);
    }

    #[test]
    fn ink_rgba_never_invents_a_colour() {
        let mut column = vec![(0u8, [0u8; 3]); 16];
        for row in 6..=9 {
            column[row] = (207, [24, 26, 43]);
        }
        let img = image_from_alpha_column(32, 16, &column);
        let p = decode_border_texture_profile(BorderMaterial::Country, "test/country.dds", &img)
            .unwrap();
        assert_eq!(p.ink_rgba(), [24, 26, 43, 207]);
        let [r, g, b, _] = p.ink_rgba();
        assert!(
            !(r > g + 40 && r > b + 40),
            "country ink must not be red-dominant: {:?}",
            p.ink_rgba()
        );
    }

    #[test]
    fn all_transparent_texture_fails_closed() {
        let column = vec![(0u8, [0u8; 3]); 8];
        let img = image_from_alpha_column(8, 8, &column);
        let err = decode_border_texture_profile(BorderMaterial::Province, "test/blank.dds", &img)
            .expect_err("a fully transparent border texture must fail closed");
        assert!(err.to_string().contains("no ink row"), "unexpected: {err}");
    }

    #[test]
    fn empty_dimensions_fail_closed() {
        let img = RgbaImage {
            width: 0,
            height: 0,
            pixels: Vec::new(),
        };
        assert!(
            decode_border_texture_profile(BorderMaterial::State, "test/empty.dds", &img).is_err()
        );
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn shipped_border_textures_match_documented_profiles() {
        let game = replay_test_gates::require_vanilla_game_root();
        let resolver = GameFileResolver::vanilla(&game).expect("vanilla resolver");
        let profiles = all_border_texture_profiles_from_resolver(&resolver)
            .expect("decode all shipped border textures");
        let by: std::collections::BTreeMap<_, _> =
            profiles.iter().map(|p| (p.material, p)).collect();

        let expect: [(BorderMaterial, usize, usize, u8); 6] = [
            (BorderMaterial::Province, 64, 32, 179),
            (BorderMaterial::State, 128, 64, 114),
            (BorderMaterial::Country, 256, 128, 207),
            (BorderMaterial::SeaCoast, 64, 32, 114),
            (BorderMaterial::SeaRegion, 64, 32, 219),
            (BorderMaterial::Impassable, 128, 64, 255),
        ];
        for (material, w, h, peak) in expect {
            let p = by.get(&material).expect("class profile present");
            assert_eq!((p.width, p.height), (w, h), "{material:?} dims");
            assert_eq!(p.peak_alpha, peak, "{material:?} peak alpha");
            let [r, g, b] = p.peak_rgb;
            assert!(
                !(r > g + 40 && r > b + 40),
                "{material:?} ink must not be red-dominant: {:?}",
                p.peak_rgb
            );
        }

        let frac = |m: BorderMaterial| by.get(&m).unwrap().ink_fraction_of_quad();
        assert!(
            frac(BorderMaterial::Province) < frac(BorderMaterial::State),
            "province ink must be thinner than state"
        );
        assert!(
            frac(BorderMaterial::State) < frac(BorderMaterial::Impassable),
            "state ink must be thinner than impassable"
        );
        assert!(
            frac(BorderMaterial::Impassable) >= 0.9,
            "impassable fills nearly the whole quad"
        );
    }
}
