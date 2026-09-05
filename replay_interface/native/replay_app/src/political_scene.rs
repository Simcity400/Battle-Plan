use map_viewer::GameFileResolver;

use crate::Result;

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
        let values = resolver.read_effective_define_numbers("NGraphics", &[
            "GRADIENT_BORDERS_OUTLINE_CUTOFF_STRATEGIC_REGIONS", "GRADIENT_BORDERS_OUTLINE_CUTOFF_COUNTRY", "DRAW_SHADOWS_CUTOFF", "DRAW_SHADOWS_FADE_LENGTH"
        ])?;
        if values.iter().any(|value| !(*value as f32).is_finite()) { return Err("nonfinite political graphics define".into()); }
        Ok(Self { outline_cutoff_strategic: values[0] as f32, outline_cutoff_country: values[1] as f32, draw_shadows_cutoff: values[2] as f32, draw_shadows_fade_length: values[3] as f32 })
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
        let root = std::env::temp_dir().join(format!("modvfs-political-defines-{}", std::process::id()));
        std::fs::create_dir_all(root.join("common/defines")).unwrap();
        std::fs::write(root.join("common/defines/fixture.lua"),
            format!("NDefines = {{ NGraphics = {{ {body} }} }}")).unwrap();
        let resolver = GameFileResolver::vanilla(&root).unwrap();
        assert_eq!(
            PoliticalSceneDefines::from_resolver(&resolver).expect("evaluate defines"),
            PoliticalSceneDefines::vanilla()
        );
        drop(resolver);
        std::fs::remove_dir_all(root).unwrap();
    }
}
