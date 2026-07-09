use map_viewer::GameFileResolver;

use crate::Result;

const GRAPHICS_DEFINES_REL: &str = "common/defines/00_graphics.lua";

/// Political-mapmode scene CB0 defines. These feed the captured pdxmap shader
/// path; they do not synthesize political textures.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PoliticalSceneDefines {
    pub(crate) outline_cutoff_strategic: f32,
    pub(crate) outline_cutoff_country: f32,
    pub(crate) draw_shadows_cutoff: f32,
    pub(crate) draw_shadows_fade_length: f32,
}

impl PoliticalSceneDefines {
    #[cfg(test)]
    pub(crate) const fn vanilla() -> Self {
        Self {
            outline_cutoff_strategic: 0.98,
            outline_cutoff_country: 0.973,
            draw_shadows_cutoff: 400.0,
            draw_shadows_fade_length: 50.0,
        }
    }

    pub(crate) fn from_resolver(resolver: &GameFileResolver) -> Result<Self> {
        let path = resolver.resolve_required(GRAPHICS_DEFINES_REL, "graphics defines lua")?;
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        Self::from_graphics_defines(&text)
    }

    fn from_graphics_defines(text: &str) -> Result<Self> {
        Ok(Self {
            outline_cutoff_strategic: parse_lua_number(
                text,
                "GRADIENT_BORDERS_OUTLINE_CUTOFF_STRATEGIC_REGIONS",
            )?,
            outline_cutoff_country: parse_lua_number(
                text,
                "GRADIENT_BORDERS_OUTLINE_CUTOFF_COUNTRY",
            )?,
            draw_shadows_cutoff: parse_lua_number(text, "DRAW_SHADOWS_CUTOFF")?,
            draw_shadows_fade_length: parse_lua_number(text, "DRAW_SHADOWS_FADE_LENGTH")?,
        })
    }

    pub(crate) fn to_cb0(self) -> map_viewer::PoliticalSceneCb0 {
        map_viewer::PoliticalSceneCb0 {
            outline_cutoff_strategic: self.outline_cutoff_strategic,
            outline_cutoff_country: self.outline_cutoff_country,
            draw_shadows_cutoff: self.draw_shadows_cutoff,
            draw_shadows_fade_length: self.draw_shadows_fade_length,
            snow_mud_fade: None,
            shadow_map_bound: false,
        }
    }
}

fn parse_lua_number(text: &str, key: &str) -> Result<f32> {
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with("--") {
            continue;
        }
        let Some((lhs, rhs)) = line.split_once('=') else {
            continue;
        };
        if lhs.trim() != key {
            continue;
        }
        let rhs = rhs.split("--").next().unwrap_or(rhs);
        let rhs = rhs.trim().trim_end_matches(',').trim();
        if let Ok(value) = rhs.parse::<f32>() {
            return Ok(value);
        }
    }
    Err(format!("{GRAPHICS_DEFINES_REL} missing {key}").into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vanilla_political_scene_defines() {
        let body = r#"
DRAW_SHADOWS_CUTOFF = 400,
DRAW_SHADOWS_FADE_LENGTH = 50,
GRADIENT_BORDERS_OUTLINE_CUTOFF_STRATEGIC_REGIONS = 0.98,
GRADIENT_BORDERS_OUTLINE_CUTOFF_COUNTRY = 0.973,
"#;
        assert_eq!(
            PoliticalSceneDefines::from_graphics_defines(body).expect("parse defines"),
            PoliticalSceneDefines::vanilla()
        );
    }
}
